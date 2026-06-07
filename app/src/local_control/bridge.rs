//! Bridge between protocol-level control requests and Warp application models.
//!
//! The bridge validates protocol version, selectors, credentials, and settings
//! before routing each supported action to an app-side handler.
use std::collections::HashMap;

use ::local_control::auth::CredentialGrant;
use ::local_control::protocol::{
    PaneTarget, TabCloseMode, TabCloseParams, TabTarget, TargetSelector,
};
use ::local_control::{
    Action, ActionKind, ControlError, ErrorCode, InstanceId, RequestEnvelope, ResponseEnvelope,
};
use chrono::{DateTime, Duration, Utc};
use futures::channel::oneshot;
use serde_json::json;
use uuid::Uuid;
use warpui::platform::TerminationMode;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity, ViewHandle, WindowId};

use crate::local_control::confirmation_dialog::LocalControlConfirmationPrompt;
use crate::local_control::handlers::{app_state, metadata, metadata_config, settings_surfaces};
use crate::local_control::permissions::{
    ensure_action_allowed, ensure_feature_enabled, ensure_protocol_version,
};
use crate::local_control::resolver::{
    target_window_id_for_target, validate_action_params, validate_action_target,
};
use crate::workspace::Workspace;

const MAX_PENDING_CONFIRMATIONS: usize = 16;
const CONFIRMATION_TTL: Duration = Duration::seconds(60);

#[derive(Debug, Clone, PartialEq, Eq)]
enum CloseBinding {
    Window {
        window_id: String,
    },
    Tabs {
        window_id: String,
        tab_ids: Vec<String>,
    },
    Pane {
        window_id: String,
        tab_id: String,
        pane_id: String,
    },
}

impl ApprovedClose {
    pub(super) fn action_kind(&self) -> ActionKind {
        self.request.action.kind
    }

    pub(super) fn credential_id(&self) -> &str {
        &self.grant.credential_id
    }

    pub(super) fn terminal_session_id(&self) -> &str {
        &self.grant.terminal_session_id
    }
}

pub(super) struct ApprovedClose {
    request: RequestEnvelope,
    grant: CredentialGrant,
    binding: CloseBinding,
    expires_at: DateTime<Utc>,
}

struct PendingConfirmation {
    approval: ApprovedClose,
    decision_sender: oneshot::Sender<Result<ApprovedClose, ControlError>>,
}

pub(super) struct PendingCloseConfirmation {
    pub confirmation_id: Uuid,
    pub decision_receiver: oneshot::Receiver<Result<ApprovedClose, ControlError>>,
}

/// WarpUI model that executes already-authenticated local-control actions.
pub struct LocalControlBridge {
    instance_id: Option<InstanceId>,
    pending_confirmations: HashMap<Uuid, PendingConfirmation>,
}

impl CloseBinding {
    fn target_summary(&self) -> String {
        match self {
            CloseBinding::Window { .. } => "Close the selected Warp window.".to_owned(),
            CloseBinding::Tabs { tab_ids, .. } => {
                format!("Close {} selected Warp tab(s).", tab_ids.len())
            }
            CloseBinding::Pane { .. } => "Close the selected Warp pane.".to_owned(),
        }
    }
}

fn resolve_close_binding(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(CloseBinding, WindowId), ControlError> {
    match request.action.kind {
        ActionKind::WindowClose => {
            validate_empty_params(&request.action)?;
            if request.target.tab.is_some()
                || request.target.pane.is_some()
                || request.target.session.is_some()
            {
                return Err(ControlError::new(
                    ErrorCode::InvalidSelector,
                    "window.close does not accept tab, pane, or session selectors",
                ));
            }
            let window_id =
                target_window_id_for_target(ctx, &request.target, ActionKind::WindowClose)?;
            Ok((
                CloseBinding::Window {
                    window_id: window_id.to_string(),
                },
                window_id,
            ))
        }
        ActionKind::TabClose => resolve_tab_close_binding(request, ctx),
        ActionKind::PaneClose => resolve_pane_close_binding(request, ctx),
        action => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!("{} is not a confirmed close action", action.as_str()),
        )),
    }
}

fn resolve_tab_close_binding(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(CloseBinding, WindowId), ControlError> {
    if request.target.pane.is_some() || request.target.session.is_some() {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "tab.close does not accept pane or session selectors",
        ));
    }
    let mode = tab_close_mode(&request.action)?;
    let window_id = target_window_id_for_target(ctx, &request.target, ActionKind::TabClose)?;
    let workspace = workspace_for_window(window_id, ActionKind::TabClose, ctx)?;
    let tab_ids = workspace.read(ctx, |workspace, ctx| {
        let all_tab_ids = workspace
            .tab_views()
            .map(|tab| tab.id().to_string())
            .collect::<Vec<_>>();
        let selected_index = tab_index_for_target(
            &request.target,
            workspace.active_tab_index(),
            &all_tab_ids,
            workspace,
            ctx,
        )?;
        let tab_ids = match mode {
            TabCloseMode::Target => vec![all_tab_ids[selected_index].clone()],
            TabCloseMode::Active => {
                if !matches!(request.target.tab.as_ref(), None | Some(TabTarget::Active)) {
                    return Err(ControlError::new(
                        ErrorCode::InvalidSelector,
                        "tab.close active does not accept a concrete tab selector",
                    ));
                }
                vec![all_tab_ids[workspace.active_tab_index()].clone()]
            }
            TabCloseMode::Others => all_tab_ids
                .into_iter()
                .enumerate()
                .filter_map(|(index, id)| (index != selected_index).then_some(id))
                .collect(),
            TabCloseMode::RightOf => all_tab_ids.into_iter().skip(selected_index + 1).collect(),
        };
        Ok::<_, ControlError>(tab_ids)
    })?;
    Ok((
        CloseBinding::Tabs {
            window_id: window_id.to_string(),
            tab_ids,
        },
        window_id,
    ))
}

fn resolve_pane_close_binding(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(CloseBinding, WindowId), ControlError> {
    validate_empty_params(&request.action)?;
    if request.target.session.is_some() {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "pane.close does not accept session selectors",
        ));
    }
    let window_id = target_window_id_for_target(ctx, &request.target, ActionKind::PaneClose)?;
    let workspace = workspace_for_window(window_id, ActionKind::PaneClose, ctx)?;
    let (tab_id, pane_id) = workspace.read(ctx, |workspace, ctx| {
        let tab_ids = workspace
            .tab_views()
            .map(|tab| tab.id().to_string())
            .collect::<Vec<_>>();
        let tab_index = tab_index_for_target(
            &request.target,
            workspace.active_tab_index(),
            &tab_ids,
            workspace,
            ctx,
        )?;
        let pane_group = workspace
            .get_pane_group_view(tab_index)
            .cloned()
            .ok_or_else(|| {
                ControlError::new(ErrorCode::StaleTarget, "pane.close target tab is stale")
            })?;
        let pane_id = pane_group.read(ctx, |pane_group, ctx| {
            let pane_ids = pane_group.visible_pane_ids();
            match request.target.pane.as_ref() {
                None | Some(PaneTarget::Active) => Ok(pane_group.focused_pane_id(ctx)),
                Some(PaneTarget::Id { id }) => pane_ids
                    .into_iter()
                    .find(|pane_id| pane_id.to_string() == id.0)
                    .ok_or_else(|| {
                        ControlError::new(
                            ErrorCode::StaleTarget,
                            "pane.close cannot resolve the requested pane id",
                        )
                    }),
                Some(PaneTarget::Index { index }) => {
                    pane_ids.into_iter().nth(*index as usize).ok_or_else(|| {
                        ControlError::new(
                            ErrorCode::StaleTarget,
                            "pane.close cannot resolve the requested pane index",
                        )
                    })
                }
            }
        })?;
        Ok::<_, ControlError>((tab_ids[tab_index].clone(), pane_id.to_string()))
    })?;
    Ok((
        CloseBinding::Pane {
            window_id: window_id.to_string(),
            tab_id,
            pane_id,
        },
        window_id,
    ))
}

fn tab_index_for_target(
    target: &TargetSelector,
    active_index: usize,
    tab_ids: &[String],
    workspace: &Workspace,
    ctx: &AppContext,
) -> Result<usize, ControlError> {
    match target.tab.as_ref() {
        None | Some(TabTarget::Active) => Ok(active_index),
        Some(TabTarget::Id { id }) => tab_ids
            .iter()
            .position(|tab_id| *tab_id == id.0)
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    "close action cannot resolve the requested tab id",
                )
            }),
        Some(TabTarget::Index { index }) => {
            let index = *index as usize;
            (index < tab_ids.len()).then_some(index).ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    "close action cannot resolve the requested tab index",
                )
            })
        }
        Some(TabTarget::Title { title }) => {
            let matches = workspace
                .tab_views()
                .enumerate()
                .filter_map(|(index, tab)| {
                    (tab.as_ref(ctx).display_title(ctx).as_str() == title).then_some(index)
                })
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [index] => Ok(*index),
                [] => Err(ControlError::new(
                    ErrorCode::MissingTarget,
                    "close action cannot resolve the requested tab title",
                )),
                _ => Err(ControlError::new(
                    ErrorCode::AmbiguousTarget,
                    "close action resolved multiple tabs by title",
                )),
            }
        }
    }
}

fn tab_close_mode(action: &Action) -> Result<TabCloseMode, ControlError> {
    Ok(action.params_as::<TabCloseParams>()?.mode)
}

fn validate_empty_params(action: &Action) -> Result<(), ControlError> {
    if action
        .params
        .as_object()
        .is_some_and(serde_json::Map::is_empty)
    {
        return Ok(());
    }
    Err(ControlError::new(
        ErrorCode::InvalidParams,
        format!("{} does not accept parameters", action.kind.as_str()),
    ))
}

fn workspace_for_window(
    window_id: WindowId,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ViewHandle<Workspace>, ControlError> {
    ctx.views_of_type::<Workspace>(window_id)
        .and_then(|workspaces| workspaces.into_iter().next())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                format!(
                    "{} requires a workspace in the target window",
                    action.as_str()
                ),
            )
        })
}

fn execute_close_binding(
    binding: &CloseBinding,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    let window_id = ctx
        .window_ids()
        .find(|window_id| window_id.to_string() == binding.window_id())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::StaleTarget,
                "the approved close window is no longer available",
            )
        })?;
    match binding {
        CloseBinding::Window { .. } => {
            ctx.windows()
                .close_window(window_id, TerminationMode::ForceTerminate);
            Ok(())
        }
        CloseBinding::Tabs { tab_ids, .. } => {
            let closed = workspace_for_window(window_id, action, ctx)?
                .update(ctx, |workspace, ctx| {
                    workspace.close_local_control_tabs(tab_ids, ctx)
                });
            if closed {
                Ok(())
            } else {
                Err(ControlError::new(
                    ErrorCode::TargetStateConflict,
                    "the approved tab close target changed before execution",
                ))
            }
        }
        CloseBinding::Pane {
            tab_id, pane_id, ..
        } => {
            let workspace = workspace_for_window(window_id, action, ctx)?;
            let pane_group = workspace.read(ctx, |workspace, _| {
                workspace
                    .tab_views()
                    .find(|tab| tab.id().to_string() == *tab_id)
                    .cloned()
            });
            let pane_group = pane_group.ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    "the approved pane close tab is no longer available",
                )
            })?;
            let pane_id = pane_group
                .read(ctx, |pane_group, _| {
                    pane_group
                        .visible_pane_ids()
                        .into_iter()
                        .find(|current| current.to_string() == *pane_id)
                })
                .ok_or_else(|| {
                    ControlError::new(
                        ErrorCode::StaleTarget,
                        "the approved pane close target is no longer available",
                    )
                })?;
            pane_group.update(ctx, |pane_group, ctx| pane_group.close_pane(pane_id, ctx));
            Ok(())
        }
    }
}

fn dismiss_confirmation_prompt(
    window_id: &str,
    confirmation_id: Uuid,
    ctx: &mut ModelContext<LocalControlBridge>,
) {
    let window_id = ctx
        .window_ids()
        .find(|candidate| candidate.to_string() == window_id);
    let Some(window_id) = window_id else {
        return;
    };
    if let Ok(workspace) = workspace_for_window(window_id, ActionKind::WindowClose, ctx) {
        workspace.update(ctx, |workspace, ctx| {
            workspace.dismiss_local_control_confirmation(confirmation_id, ctx);
        });
    }
}

impl CloseBinding {
    fn window_id(&self) -> &str {
        match self {
            CloseBinding::Window { window_id }
            | CloseBinding::Tabs { window_id, .. }
            | CloseBinding::Pane { window_id, .. } => window_id,
        }
    }
}

impl Entity for LocalControlBridge {
    type Event = ();
}

impl SingletonEntity for LocalControlBridge {}

impl LocalControlBridge {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            instance_id: None,
            pending_confirmations: HashMap::new(),
        }
    }

    pub(super) fn set_instance_id(&mut self, instance_id: InstanceId) {
        self.instance_id = Some(instance_id);
    }

    pub(super) fn prepare_close_confirmation(
        &mut self,
        request: RequestEnvelope,
        grant: CredentialGrant,
        ctx: &mut ModelContext<Self>,
    ) -> Result<PendingCloseConfirmation, ControlError> {
        ensure_feature_enabled()?;
        ensure_protocol_version(request.protocol_version)?;
        let instance_id = self.instance_id.as_ref().ok_or_else(|| {
            ControlError::new(
                ErrorCode::BridgeUnavailable,
                "local-control bridge has no active instance identity",
            )
        })?;
        validate_request_authority(instance_id, &request.action, &grant)?;
        ensure_action_allowed(request.action.kind, ctx)?;
        if !request.action.kind.metadata().requires_user_confirmation {
            return Err(ControlError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "{} does not require local-control confirmation",
                    request.action.kind.as_str()
                ),
            ));
        }
        if self.pending_confirmations.len() >= MAX_PENDING_CONFIRMATIONS {
            return Err(ControlError::new(
                ErrorCode::UserConfirmationRequired,
                "too many local-control confirmation requests are pending",
            ));
        }
        let (binding, window_id) = resolve_close_binding(&request, ctx)?;
        let workspace = workspace_for_window(window_id, request.action.kind, ctx)?;
        let confirmation_id = Uuid::new_v4();
        let expires_at = Utc::now() + CONFIRMATION_TTL;
        let approval = ApprovedClose {
            request: request.clone(),
            grant,
            binding: binding.clone(),
            expires_at,
        };
        let (decision_sender, decision_receiver) = oneshot::channel();
        self.pending_confirmations.insert(
            confirmation_id,
            PendingConfirmation {
                approval,
                decision_sender,
            },
        );
        let prompt = LocalControlConfirmationPrompt {
            confirmation_id,
            action: request.action.kind,
            target_summary: binding.target_summary(),
        };
        let shown = workspace.update(ctx, |workspace, ctx| {
            workspace.show_local_control_confirmation(prompt, ctx)
        });
        if !shown {
            self.pending_confirmations.remove(&confirmation_id);
            return Err(ControlError::new(
                ErrorCode::UserConfirmationRequired,
                "the target window is already showing a local-control confirmation",
            ));
        }
        ctx.windows().show_window_and_focus_app(window_id);
        Ok(PendingCloseConfirmation {
            confirmation_id,
            decision_receiver,
        })
    }

    pub(crate) fn resolve_confirmation(&mut self, confirmation_id: Uuid, approved: bool) {
        let Some(pending) = self.pending_confirmations.remove(&confirmation_id) else {
            return;
        };
        let decision = if Utc::now() >= pending.approval.expires_at {
            Err(ControlError::new(
                ErrorCode::UserConfirmationExpired,
                "local-control confirmation expired",
            ))
        } else if approved {
            Ok(pending.approval)
        } else {
            Err(ControlError::new(
                ErrorCode::UserConfirmationDenied,
                "local-control close request was denied",
            ))
        };
        let _ = pending.decision_sender.send(decision);
    }

    pub(super) fn cancel_confirmation(
        &mut self,
        confirmation_id: Uuid,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(pending) = self.pending_confirmations.remove(&confirmation_id) {
            dismiss_confirmation_prompt(pending.approval.binding.window_id(), confirmation_id, ctx);
            let _ = pending.decision_sender.send(Err(ControlError::new(
                ErrorCode::UserConfirmationExpired,
                "local-control confirmation expired",
            )));
        }
    }

    pub(super) fn cancel_all_confirmations(&mut self, ctx: &mut ModelContext<Self>) {
        let pending_confirmations = self.pending_confirmations.drain().collect::<Vec<_>>();
        for (confirmation_id, pending) in pending_confirmations {
            dismiss_confirmation_prompt(pending.approval.binding.window_id(), confirmation_id, ctx);
            let _ = pending.decision_sender.send(Err(ControlError::new(
                ErrorCode::UserConfirmationExpired,
                "local-control confirmation was revoked",
            )));
        }
    }

    pub(super) fn execute_approved_close(
        &mut self,
        approval: ApprovedClose,
        ctx: &mut ModelContext<Self>,
    ) -> ResponseEnvelope {
        let request_id = approval.request.request_id;
        let result = self.execute_approved_close_inner(approval, ctx);
        match result {
            Ok(data) => ResponseEnvelope::ok(request_id, data),
            Err(error) => ResponseEnvelope::error(request_id, error),
        }
    }

    fn execute_approved_close_inner(
        &mut self,
        approval: ApprovedClose,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        if Utc::now() >= approval.expires_at {
            return Err(ControlError::new(
                ErrorCode::UserConfirmationExpired,
                "local-control confirmation expired",
            ));
        }
        ensure_feature_enabled()?;
        let instance_id = self.instance_id.as_ref().ok_or_else(|| {
            ControlError::new(
                ErrorCode::BridgeUnavailable,
                "local-control bridge has no active instance identity",
            )
        })?;
        validate_request_authority(instance_id, &approval.request.action, &approval.grant)?;
        ensure_action_allowed(approval.request.action.kind, ctx)?;
        let (current_binding, _) = resolve_close_binding(&approval.request, ctx)?;
        if current_binding != approval.binding {
            return Err(ControlError::new(
                ErrorCode::TargetStateConflict,
                "the approved close target changed before execution",
            ));
        }
        execute_close_binding(&approval.binding, approval.request.action.kind, ctx)?;
        Ok(json!({
            "action": approval.request.action.kind.as_str(),
            "ok": true,
        }))
    }

    pub(super) fn handle_request(
        &mut self,
        request: RequestEnvelope,
        grant: CredentialGrant,
        ctx: &mut ModelContext<Self>,
    ) -> ResponseEnvelope {
        if let Err(error) = ensure_feature_enabled() {
            return ResponseEnvelope::error(request.request_id, error);
        }
        if let Err(error) = ensure_protocol_version(request.protocol_version) {
            return ResponseEnvelope::error(request.request_id, error);
        }
        let Some(instance_id) = &self.instance_id else {
            return ResponseEnvelope::error(
                request.request_id,
                ControlError::new(
                    ErrorCode::BridgeUnavailable,
                    "local-control bridge has no active instance identity",
                ),
            );
        };
        if let Err(error) = validate_request_authority(instance_id, &request.action, &grant) {
            return ResponseEnvelope::error(request.request_id, error);
        }
        if let Err(error) = ensure_action_allowed(request.action.kind, ctx) {
            return ResponseEnvelope::error(request.request_id, error);
        }
        if let Err(error) = validate_action_target(request.action.kind, &request.target) {
            return ResponseEnvelope::error(request.request_id, error);
        }
        match request.action.kind {
            ActionKind::InstanceList => match metadata::instance(&self.instance_id) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::InstanceInspect => match metadata::inspect(&self.instance_id, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::AppPing => match metadata::ping(&self.instance_id) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::AppVersion => match metadata::version(&self.instance_id) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::AppActive => match metadata::active(&self.instance_id, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::CapabilityList => {
                ResponseEnvelope::ok(request.request_id, metadata::capability_list())
            }
            ActionKind::CapabilityInspect => match metadata::capability_inspect(&request.action) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::ActionList => {
                ResponseEnvelope::ok(request.request_id, metadata::action_list())
            }
            ActionKind::ActionInspect => match metadata::action_inspect(&request.action) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::WindowList => match metadata::window_list(&request.target, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::WindowInspect => match metadata::window_inspect(&request.target, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::TabList => match metadata::tab_list(&request.target, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::TabInspect => match metadata::tab_inspect(&request.target, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::AppFocus
            | ActionKind::WindowCreate
            | ActionKind::WindowFocus
            | ActionKind::TabCreate
            | ActionKind::TabActivate
            | ActionKind::TabMove
            | ActionKind::PaneSplit
            | ActionKind::PaneFocus
            | ActionKind::PaneNavigate
            | ActionKind::PaneResize
            | ActionKind::PaneMaximize
            | ActionKind::PaneUnmaximize
            | ActionKind::SessionActivate
            | ActionKind::SessionPrevious
            | ActionKind::SessionNext
            | ActionKind::SessionReopenClosed
            | ActionKind::InputInsert
            | ActionKind::InputReplace
            | ActionKind::SurfaceSettingsOpen
            | ActionKind::SurfaceCommandPaletteOpen
            | ActionKind::SurfaceCommandSearchOpen
            | ActionKind::SurfaceWarpDriveOpen
            | ActionKind::SurfaceWarpDriveToggle
            | ActionKind::SurfaceResourceCenterToggle
            | ActionKind::SurfaceAiAssistantToggle
            | ActionKind::SurfaceCodeReviewToggle
            | ActionKind::SurfaceLeftPanelToggle
            | ActionKind::SurfaceRightPanelToggle
            | ActionKind::SurfaceVerticalTabsToggle
            | ActionKind::FileOpen => {
                match app_state::handle(
                    &self.instance_id,
                    request.action.kind,
                    &request.action.params,
                    &request.target,
                    ctx,
                ) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::TabRename => match metadata_config::tab_rename(
                &self.instance_id,
                &request.target,
                &request.action,
                ctx,
            ) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::TabResetName => {
                match metadata_config::tab_reset_name(&self.instance_id, &request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::TabColorSet => match metadata_config::tab_color_set(
                &self.instance_id,
                &request.target,
                &request.action,
                ctx,
            ) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::TabColorClear => {
                match metadata_config::tab_color_clear(&self.instance_id, &request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::PaneList => match metadata::pane_list(&request.target, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::PaneInspect => match metadata::pane_inspect(&request.target, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::PaneRename => match metadata_config::pane_rename(
                &self.instance_id,
                &request.target,
                &request.action,
                ctx,
            ) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::PaneResetName => {
                match metadata_config::pane_reset_name(&self.instance_id, &request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::SessionList => match metadata::session_list(&request.target, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::SessionInspect => match metadata::session_inspect(&request.target, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::BlockList => {
                match metadata::block_list(&request.action, &request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::ThemeList => match settings_surfaces::theme_list(ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::ThemeGet => match settings_surfaces::theme_get(ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::ThemeSet
            | ActionKind::ThemeSystemSet
            | ActionKind::ThemeLightSet
            | ActionKind::ThemeDarkSet => {
                match metadata_config::theme_set(request.action.kind, &request.action, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::AppearanceGet => match settings_surfaces::appearance_get(ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::AppearanceFontSizeIncrease
            | ActionKind::AppearanceFontSizeDecrease
            | ActionKind::AppearanceFontSizeReset
            | ActionKind::AppearanceZoomIncrease
            | ActionKind::AppearanceZoomDecrease
            | ActionKind::AppearanceZoomReset => {
                match metadata_config::appearance_mutation(request.action.kind, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::SettingList => {
                match settings_surfaces::setting_list(&request.action, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::SettingGet => match settings_surfaces::setting_get(&request.action, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::SettingSet => match metadata_config::setting_set(&request.action, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::SettingToggle => {
                match metadata_config::setting_toggle(&request.action, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::KeybindingList => match settings_surfaces::keybinding_list(ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::KeybindingGet => {
                match settings_surfaces::keybinding_get(&request.action, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::WindowClose | ActionKind::TabClose | ActionKind::PaneClose => {
                ResponseEnvelope::error(
                    request.request_id,
                    ControlError::new(
                        ErrorCode::UserConfirmationRequired,
                        format!(
                            "{} requires exact-request one-shot confirmation",
                            request.action.kind.as_str()
                        ),
                    ),
                )
            }
        }
    }
}

pub(crate) fn validate_request_authority(
    instance_id: &InstanceId,
    action: &Action,
    grant: &CredentialGrant,
) -> Result<(), ControlError> {
    grant.verify_for_action(instance_id, action.kind)?;
    if !action.kind.is_implemented() {
        return Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!(
                "{} is not implemented by this local-control bridge",
                action.kind.as_str()
            ),
        ));
    }
    validate_action_params(action)
}
