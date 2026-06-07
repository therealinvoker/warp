//! Safe app-state mutation and visible UI intent handlers for local-control actions.
#[cfg(test)]
#[path = "app_state_tests.rs"]
mod tests;

#[cfg(feature = "local_fs")]
use std::path::PathBuf;
use std::str::FromStr;

use ::local_control::protocol::{
    Direction as ControlDirection, DirectionParams, FileOpenParams, PageQueryParams, PaneTarget,
    QueryParams, ResizeParams, SessionTarget, TabActivateParams, TabActivationMode,
    TabCreateParams, TabTarget, TabType, TargetSelector, TextParams,
};
use ::local_control::{ActionKind, ControlError, ErrorCode, InstanceId};
use serde_json::json;
use warp_util::path::LineAndColumnArg;
#[cfg(feature = "local_fs")]
use warpui::SingletonEntity;
use warpui::{AppContext, ModelContext, TypedActionView, ViewHandle};

#[cfg(feature = "local_fs")]
use crate::code::editor_management::CodeSource;
use crate::local_control::handlers::layout::{create_tab, resolve_shell};
use crate::local_control::resolver::target_window_id_for_target;
use crate::local_control::LocalControlBridge;
use crate::palette::PaletteMode;
use crate::pane_group::{ActivationReason, Direction, PaneGroup, PaneGroupAction, PaneId};
use crate::server::telemetry::PaletteSource;
use crate::settings_view::SettingsSection;
#[cfg(feature = "local_fs")]
use crate::util::file::external_editor::EditorSettings;
#[cfg(feature = "local_fs")]
use crate::util::openable_file_type::{resolve_file_target_to_open_in_warp, EditorLayout};
use crate::workspace::{CommandSearchOptions, InitContent, Workspace, WorkspaceAction};

const MAX_PANE_RESIZE_STEPS: u32 = 1_000;

pub(crate) fn handle(
    instance_id: &Option<InstanceId>,
    action: ActionKind,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    match action {
        ActionKind::AppFocus | ActionKind::WindowFocus => {
            focus_window(instance_id, action, target, ctx)
        }
        ActionKind::WindowCreate => window_create(instance_id, params, target, ctx),
        ActionKind::TabCreate => create_tab(instance_id, params, target, ctx),
        ActionKind::TabActivate => tab_activate(instance_id, params, target, ctx),
        ActionKind::TabMove => tab_move(instance_id, params, target, ctx),
        ActionKind::PaneSplit => pane_direction_action(instance_id, action, params, target, ctx),
        ActionKind::PaneFocus | ActionKind::SessionActivate => {
            pane_focus(instance_id, action, target, ctx)
        }
        ActionKind::PaneNavigate => pane_direction_action(instance_id, action, params, target, ctx),
        ActionKind::PaneResize => pane_resize(instance_id, params, target, ctx),
        ActionKind::PaneMaximize => pane_maximize(instance_id, true, target, ctx),
        ActionKind::PaneUnmaximize => pane_maximize(instance_id, false, target, ctx),
        ActionKind::SessionPrevious => workspace_action(
            instance_id,
            action,
            WorkspaceAction::CyclePrevSession,
            target,
            ctx,
        ),
        ActionKind::SessionNext => workspace_action(
            instance_id,
            action,
            WorkspaceAction::CycleNextSession,
            target,
            ctx,
        ),
        ActionKind::SessionReopenClosed => session_reopen_closed(instance_id, target, ctx),
        ActionKind::InputInsert => input_text(instance_id, action, params, target, false, ctx),
        ActionKind::InputReplace => input_text(instance_id, action, params, target, true, ctx),
        ActionKind::SurfaceSettingsOpen => surface_settings_open(instance_id, params, target, ctx),
        ActionKind::SurfaceCommandPaletteOpen => surface_palette_open(
            instance_id,
            action,
            PaletteMode::Command,
            params,
            target,
            ctx,
        ),
        ActionKind::SurfaceCommandSearchOpen => {
            surface_command_search_open(instance_id, params, target, ctx)
        }
        ActionKind::SurfaceWarpDriveOpen => workspace_action(
            instance_id,
            action,
            WorkspaceAction::OpenWarpDrive,
            target,
            ctx,
        ),
        ActionKind::SurfaceWarpDriveToggle => workspace_action(
            instance_id,
            action,
            WorkspaceAction::ToggleWarpDrive,
            target,
            ctx,
        ),
        ActionKind::SurfaceResourceCenterToggle => workspace_action(
            instance_id,
            action,
            WorkspaceAction::ToggleResourceCenter,
            target,
            ctx,
        ),
        ActionKind::SurfaceAiAssistantToggle => workspace_action(
            instance_id,
            action,
            WorkspaceAction::ToggleAIAssistant,
            target,
            ctx,
        ),
        ActionKind::SurfaceCodeReviewToggle | ActionKind::SurfaceRightPanelToggle => {
            workspace_action(
                instance_id,
                action,
                WorkspaceAction::ToggleRightPanel,
                target,
                ctx,
            )
        }
        ActionKind::SurfaceLeftPanelToggle => workspace_action(
            instance_id,
            action,
            WorkspaceAction::ToggleLeftPanel,
            target,
            ctx,
        ),
        ActionKind::SurfaceVerticalTabsToggle => workspace_action(
            instance_id,
            action,
            WorkspaceAction::ToggleVerticalTabsPanel,
            target,
            ctx,
        ),
        ActionKind::FileOpen => file_open(instance_id, params, target, ctx),
        _ => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!("{} is not a safe app-state handler action", action.as_str()),
        )),
    }
}

fn reject_target_families(
    action: ActionKind,
    rejected: bool,
    families: &str,
) -> Result<(), ControlError> {
    if rejected {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{} does not accept {families}", action.as_str()),
        ));
    }
    Ok(())
}

fn focus_window(
    instance_id: &Option<InstanceId>,
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        action,
        target.tab.is_some() || target.pane.is_some() || target.session.is_some(),
        "tab, pane, or session selectors",
    )?;
    let window_id = target_window_id_for_target(ctx, target, action)?;
    ctx.windows().show_window_and_focus_app(window_id);
    Ok(ack(instance_id, action))
}

fn window_create(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        ActionKind::WindowCreate,
        target.window.is_some()
            || target.tab.is_some()
            || target.pane.is_some()
            || target.session.is_some(),
        "target selectors",
    )?;
    let params = decode_params::<TabCreateParams>(params)?;
    match params.tab_type {
        None | Some(TabType::Terminal | TabType::Default) => {}
        Some(TabType::Agent | TabType::CloudAgent) => {
            return Err(ControlError::new(
                ErrorCode::UnsupportedAction,
                "window.create only supports terminal or default window types",
            ));
        }
    }
    match params.shell.as_deref() {
        Some(shell_name) => {
            let shell = resolve_shell(shell_name, ctx)?;
            ctx.dispatch_global_action("root_view:open_new_with_shell", Some(shell));
        }
        None => ctx.dispatch_global_action("root_view:open_new", ()),
    }
    Ok(ack(instance_id, ActionKind::WindowCreate))
}

fn workspace_action(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    action: WorkspaceAction,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let workspace = target_workspace(action_kind, target, ctx)?;
    activate_target(&workspace, action_kind, target, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        workspace.handle_action(&action, ctx);
    });
    Ok(ack(instance_id, action_kind))
}

fn session_reopen_closed(
    instance_id: &Option<InstanceId>,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        ActionKind::SessionReopenClosed,
        target.tab.is_some() || target.pane.is_some() || target.session.is_some(),
        "tab, pane, or session selectors",
    )?;
    let window_id = target_window_id_for_target(ctx, target, ActionKind::SessionReopenClosed)?;
    ctx.windows().show_window_and_focus_app(window_id);
    workspace_action(
        instance_id,
        ActionKind::SessionReopenClosed,
        WorkspaceAction::ReopenClosedSession,
        target,
        ctx,
    )
}

fn tab_activate(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        ActionKind::TabActivate,
        target.pane.is_some() || target.session.is_some(),
        "pane or session selectors",
    )?;
    let mode = decode_params::<TabActivateParams>(params)?.mode;
    if !matches!(mode, TabActivationMode::Target)
        && !matches!(target.tab.as_ref(), None | Some(TabTarget::Active))
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "tab.activate navigation modes do not accept a concrete tab selector",
        ));
    }
    let workspace = target_workspace(ActionKind::TabActivate, target, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        let action = match mode {
            TabActivationMode::Target => {
                WorkspaceAction::ActivateTab(tab_index_from_target(target, workspace, ctx)?)
            }
            TabActivationMode::Previous => WorkspaceAction::ActivatePrevTab,
            TabActivationMode::Next => WorkspaceAction::ActivateNextTab,
            TabActivationMode::Last => WorkspaceAction::ActivateLastTab,
        };
        workspace.handle_action(&action, ctx);
        Ok::<_, ControlError>(())
    })?;
    Ok(ack(instance_id, ActionKind::TabActivate))
}

fn tab_move(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        ActionKind::TabMove,
        target.pane.is_some() || target.session.is_some(),
        "pane or session selectors",
    )?;
    let direction = direction_param(params)?;
    let workspace = target_workspace(ActionKind::TabMove, target, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        let index = tab_index_from_target(target, workspace, ctx)?;
        let action = match direction {
            ControlDirection::Left => WorkspaceAction::MoveTabLeft(index),
            ControlDirection::Right => WorkspaceAction::MoveTabRight(index),
            ControlDirection::Up
            | ControlDirection::Down
            | ControlDirection::Previous
            | ControlDirection::Next => {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "tab.move only accepts left or right",
                ));
            }
        };
        workspace.handle_action(&action, ctx);
        Ok::<_, ControlError>(())
    })?;
    Ok(ack(instance_id, ActionKind::TabMove))
}

fn pane_direction_action(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let direction = direction_param(params)?;
    let action = match action_kind {
        ActionKind::PaneSplit => PaneGroupAction::Add(pane_direction(direction)?),
        ActionKind::PaneNavigate => match direction {
            ControlDirection::Left => PaneGroupAction::NavigateLeft,
            ControlDirection::Right => PaneGroupAction::NavigateRight,
            ControlDirection::Up => PaneGroupAction::NavigateUp,
            ControlDirection::Down => PaneGroupAction::NavigateDown,
            ControlDirection::Previous => PaneGroupAction::NavigatePrev,
            ControlDirection::Next => PaneGroupAction::NavigateNext,
        },
        _ => return invalid_params(action_kind),
    };
    pane_group_action(instance_id, action_kind, target, action, 1, ctx)
}

fn pane_focus(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    if target.pane.is_none()
        && (action_kind != ActionKind::SessionActivate || target.session.is_none())
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{} requires a pane or session target", action_kind.as_str()),
        ));
    }
    let pane_group = active_target_pane_group(action_kind, target, ctx)?;
    let pane_id = if action_kind == ActionKind::SessionActivate {
        target_session_pane_id(action_kind, target, &pane_group, ctx)?
    } else {
        target_pane_id(action_kind, target, &pane_group, ctx)?
    };
    pane_group.update(ctx, |pane_group, ctx| {
        pane_group.handle_action(
            &PaneGroupAction::Activate(pane_id, ActivationReason::Click),
            ctx,
        );
    });
    Ok(ack(instance_id, action_kind))
}

fn pane_resize(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let ResizeParams { direction, amount } = decode_params(params)?;
    let amount = amount.unwrap_or(1);
    if amount > MAX_PANE_RESIZE_STEPS {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("pane.resize amount cannot exceed {MAX_PANE_RESIZE_STEPS}"),
        ));
    }
    let action = match direction {
        ControlDirection::Left => PaneGroupAction::ResizeLeft,
        ControlDirection::Right => PaneGroupAction::ResizeRight,
        ControlDirection::Up => PaneGroupAction::ResizeUp,
        ControlDirection::Down => PaneGroupAction::ResizeDown,
        ControlDirection::Previous | ControlDirection::Next => {
            return Err(ControlError::new(
                ErrorCode::InvalidParams,
                "pane.resize only accepts left, right, up, or down",
            ));
        }
    };
    pane_group_action(
        instance_id,
        ActionKind::PaneResize,
        target,
        action,
        amount,
        ctx,
    )
}

fn pane_maximize(
    instance_id: &Option<InstanceId>,
    should_maximize: bool,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let action_kind = if should_maximize {
        ActionKind::PaneMaximize
    } else {
        ActionKind::PaneUnmaximize
    };
    let pane_group = active_target_pane_group(action_kind, target, ctx)?;
    focus_explicit_pane_target(action_kind, target, &pane_group, ctx)?;
    let is_maximized = pane_group.read(ctx, |pane_group, ctx| {
        pane_group.is_focused_pane_maximized(ctx)
    });
    if is_maximized != should_maximize {
        pane_group.update(ctx, |pane_group, ctx| {
            pane_group.handle_action(&PaneGroupAction::ToggleMaximizePane, ctx);
        });
    }
    Ok(ack(instance_id, action_kind))
}

fn pane_group_action(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    target: &TargetSelector,
    action: PaneGroupAction,
    repetitions: u32,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let pane_group = active_target_pane_group(action_kind, target, ctx)?;
    focus_explicit_pane_target(action_kind, target, &pane_group, ctx)?;
    pane_group.update(ctx, |pane_group, ctx| {
        for _ in 0..repetitions {
            pane_group.handle_action(&action, ctx);
        }
    });
    Ok(ack(instance_id, action_kind))
}

fn input_text(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    params: &serde_json::Value,
    target: &TargetSelector,
    replace_buffer: bool,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let text = text_param(params)?;
    validate_staged_input_text(action_kind, &text)?;
    let pane_group = target_pane_group(action_kind, target, ctx)?;
    let pane_id = input_target_pane_id(action_kind, target, &pane_group, ctx)?;
    let terminal_view = pane_group
        .read(ctx, |pane_group, ctx| {
            pane_group.terminal_view_from_pane_id(pane_id, ctx)
        })
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                format!("{} requires a terminal input target", action_kind.as_str()),
            )
        })?;
    terminal_view.update(ctx, |terminal_view, ctx| {
        terminal_view.input().update(ctx, |input, ctx| {
            if replace_buffer {
                input.replace_buffer_content(&text, ctx);
            } else {
                input.append_to_buffer(&text, ctx);
            }
        });
    });
    Ok(ack(instance_id, action_kind))
}

fn validate_staged_input_text(action: ActionKind, text: &str) -> Result<(), ControlError> {
    if text
        .chars()
        .any(|character| character == '\n' || character == '\r' || character.is_control())
    {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!(
                "{} rejects newlines, carriage returns, and control characters",
                action.as_str()
            ),
        ));
    }
    Ok(())
}

fn surface_settings_open(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let PageQueryParams { page, query } = decode_params(params)?;
    let section = page.map(settings_section).transpose()?;
    let action = match (section, query) {
        (Some(section), Some(search_query)) => WorkspaceAction::ShowSettingsPageWithSearch {
            search_query,
            section: Some(section),
        },
        (Some(section), None) => WorkspaceAction::ShowSettingsPage(section),
        (None, Some(search_query)) => WorkspaceAction::ShowSettingsPageWithSearch {
            search_query,
            section: None,
        },
        (None, None) => WorkspaceAction::ShowSettings,
    };
    workspace_action(
        instance_id,
        ActionKind::SurfaceSettingsOpen,
        action,
        target,
        ctx,
    )
}

fn settings_section(page: String) -> Result<SettingsSection, ControlError> {
    let section = SettingsSection::from_str(&page).map_err(|_| {
        ControlError::new(
            ErrorCode::InvalidParams,
            format!("surface.settings.open cannot resolve settings page {page:?}"),
        )
    })?;
    if section == SettingsSection::WarpDrive {
        return Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            "surface.settings.open does not open Warp Drive settings",
        ));
    }
    Ok(section)
}

fn surface_palette_open(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    mode: PaletteMode,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let query = decode_params::<QueryParams>(params)?.query;
    workspace_action(
        instance_id,
        action_kind,
        WorkspaceAction::OpenPalette {
            mode,
            source: PaletteSource::Keybinding,
            query,
        },
        target,
        ctx,
    )
}

fn surface_command_search_open(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let query = decode_params::<QueryParams>(params)?.query;
    let init_content = query
        .map(InitContent::Custom)
        .unwrap_or(InitContent::FromInputBuffer);
    workspace_action(
        instance_id,
        ActionKind::SurfaceCommandSearchOpen,
        WorkspaceAction::ShowCommandSearch(CommandSearchOptions {
            filter: None,
            init_content,
        }),
        target,
        ctx,
    )
}

fn file_open(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let params = decode_params::<FileOpenParams>(params)?;
    if params.path.is_empty() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "file.open requires a non-empty path",
        ));
    }
    let line_and_column = line_and_column(&params)?;
    let workspace = target_workspace(ActionKind::FileOpen, target, ctx)?;
    activate_target(&workspace, ActionKind::FileOpen, target, ctx)?;
    #[cfg(feature = "local_fs")]
    {
        let path = PathBuf::from(params.path);
        let layout = params.new_tab.then_some(EditorLayout::NewTab);
        let file_target =
            resolve_file_target_to_open_in_warp(&path, EditorSettings::as_ref(ctx), layout);
        workspace.update(ctx, |workspace, ctx| {
            workspace.open_file_with_target(
                path.clone(),
                file_target,
                line_and_column,
                CodeSource::Link {
                    path,
                    range_start: None,
                    range_end: None,
                },
                ctx,
            );
        });
        Ok(ack(instance_id, ActionKind::FileOpen))
    }
    #[cfg(not(feature = "local_fs"))]
    Err(ControlError::new(
        ErrorCode::UnsupportedAction,
        "file.open is unavailable without local filesystem support",
    ))
}

fn target_workspace(
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ViewHandle<Workspace>, ControlError> {
    let window_id = target_window_id_for_target(ctx, target, action)?;
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

fn target_pane_group(
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ViewHandle<PaneGroup>, ControlError> {
    let workspace = target_workspace(action, target, ctx)?;
    workspace.read(ctx, |workspace, ctx| {
        let tab_index = tab_index_from_target(target, workspace, ctx)?;
        workspace
            .tab_views()
            .nth(tab_index)
            .cloned()
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    format!("{} cannot resolve the requested tab", action.as_str()),
                )
            })
    })
}

fn active_target_pane_group(
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ViewHandle<PaneGroup>, ControlError> {
    reject_target_families(
        action,
        action != ActionKind::SessionActivate && target.session.is_some(),
        "session selectors",
    )?;
    let workspace = target_workspace(action, target, ctx)?;
    if target.tab.is_some() {
        workspace.update(ctx, |workspace, ctx| {
            let tab_index = tab_index_from_target(target, workspace, ctx)?;
            workspace.handle_action(&WorkspaceAction::ActivateTab(tab_index), ctx);
            Ok::<_, ControlError>(())
        })?;
    }
    target_pane_group(action, target, ctx)
}

fn activate_target(
    workspace: &ViewHandle<Workspace>,
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    if target.tab.is_some() {
        workspace.update(ctx, |workspace, ctx| {
            let tab_index = tab_index_from_target(target, workspace, ctx)?;
            workspace.handle_action(&WorkspaceAction::ActivateTab(tab_index), ctx);
            Ok::<_, ControlError>(())
        })?;
    }
    if target.session.is_some() {
        let pane_group = target_pane_group(action, target, ctx)?;
        let pane_id = target_session_pane_id(action, target, &pane_group, ctx)?;
        pane_group.update(ctx, |pane_group, ctx| {
            pane_group.handle_action(
                &PaneGroupAction::Activate(pane_id, ActivationReason::Click),
                ctx,
            );
        });
    } else if target.pane.is_some() {
        let pane_group = target_pane_group(action, target, ctx)?;
        focus_explicit_pane_target(action, target, &pane_group, ctx)?;
    }
    Ok(())
}

fn focus_explicit_pane_target(
    action: ActionKind,
    target: &TargetSelector,
    pane_group: &ViewHandle<PaneGroup>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    if target.pane.is_none() {
        return Ok(());
    }
    let pane_id = target_pane_id(action, target, pane_group, ctx)?;
    pane_group.update(ctx, |pane_group, ctx| {
        pane_group.handle_action(
            &PaneGroupAction::Activate(pane_id, ActivationReason::Click),
            ctx,
        );
    });
    Ok(())
}

fn target_pane_id(
    action: ActionKind,
    target: &TargetSelector,
    pane_group: &ViewHandle<PaneGroup>,
    ctx: &AppContext,
) -> Result<PaneId, ControlError> {
    pane_group.read(ctx, |pane_group, ctx| match target.pane.as_ref() {
        None | Some(PaneTarget::Active) => Ok(pane_group.focused_pane_id(ctx)),
        Some(PaneTarget::Index { index }) => {
            let pane_index = usize::try_from(*index).map_err(|err| {
                ControlError::with_details(
                    ErrorCode::InvalidSelector,
                    "pane index is out of range",
                    err.to_string(),
                )
            })?;
            pane_group.pane_id_from_index(pane_index).ok_or_else(|| {
                ControlError::new(
                    ErrorCode::MissingTarget,
                    format!(
                        "{} cannot resolve the requested pane index",
                        action.as_str()
                    ),
                )
            })
        }
        Some(PaneTarget::Id { id }) => pane_group
            .visible_pane_ids()
            .into_iter()
            .find(|pane_id| pane_id.to_string() == id.0)
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    format!("{} cannot resolve the requested pane id", action.as_str()),
                )
            }),
    })
}

fn tab_index_from_target(
    target: &TargetSelector,
    workspace: &Workspace,
    ctx: &AppContext,
) -> Result<usize, ControlError> {
    match target.tab.as_ref() {
        Some(TabTarget::Index { index }) => usize::try_from(*index)
            .ok()
            .filter(|index| *index < workspace.tab_count())
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::MissingTarget,
                    "tab selector index did not match a visible tab",
                )
            }),
        Some(TabTarget::Active) | None => Ok(workspace.active_tab_index()),
        Some(TabTarget::Id { id }) => workspace
            .tab_views()
            .enumerate()
            .find_map(|(index, pane_group)| (pane_group.id().to_string() == id.0).then_some(index))
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    "tab selector id did not match a visible tab",
                )
            }),
        Some(TabTarget::Title { title }) => {
            let matching = workspace
                .tab_views()
                .enumerate()
                .filter_map(|(index, pane_group)| {
                    (pane_group.as_ref(ctx).display_title(ctx) == *title).then_some(index)
                })
                .collect::<Vec<_>>();
            match matching.as_slice() {
                [index] => Ok(*index),
                [] => Err(ControlError::new(
                    ErrorCode::MissingTarget,
                    "tab selector title did not match a visible tab",
                )),
                _ => Err(ControlError::new(
                    ErrorCode::AmbiguousTarget,
                    "tab selector title matched multiple visible tabs",
                )),
            }
        }
    }
}

fn decode_params<T: serde::de::DeserializeOwned>(
    params: &serde_json::Value,
) -> Result<T, ControlError> {
    serde_json::from_value(params.clone()).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidParams,
            "failed to decode action parameters",
            err.to_string(),
        )
    })
}

fn direction_param(params: &serde_json::Value) -> Result<ControlDirection, ControlError> {
    Ok(decode_params::<DirectionParams>(params)?.direction)
}
fn text_param(params: &serde_json::Value) -> Result<String, ControlError> {
    Ok(decode_params::<TextParams>(params)?.text)
}

fn input_target_pane_id(
    action: ActionKind,
    target: &TargetSelector,
    pane_group: &ViewHandle<PaneGroup>,
    ctx: &AppContext,
) -> Result<PaneId, ControlError> {
    if target.session.is_some() {
        return target_session_pane_id(action, target, pane_group, ctx);
    }
    if target.pane.is_some() {
        return target_pane_id(action, target, pane_group, ctx);
    }
    pane_group
        .read(ctx, |pane_group, ctx| {
            pane_group.active_session_id(ctx).map(PaneId::from)
        })
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                format!("{} requires an active terminal session", action.as_str()),
            )
        })
}

fn target_session_pane_id(
    action: ActionKind,
    target: &TargetSelector,
    pane_group: &ViewHandle<PaneGroup>,
    ctx: &AppContext,
) -> Result<PaneId, ControlError> {
    if target.session.is_none() && target.pane.is_some() {
        let pane_id = target_pane_id(action, target, pane_group, ctx)?;
        let has_terminal = pane_group.read(ctx, |pane_group, ctx| {
            pane_group
                .terminal_view_from_pane_id(pane_id, ctx)
                .is_some()
        });
        if has_terminal {
            return Ok(pane_id);
        }
        return Err(ControlError::new(
            ErrorCode::MissingTarget,
            format!("{} requires a terminal session target", action.as_str()),
        ));
    }
    let session_pane_id =
        pane_group.read(ctx, |pane_group, ctx| match target.session.as_ref() {
            None | Some(SessionTarget::Active) => pane_group
                .active_session_id(ctx)
                .map(PaneId::from)
                .ok_or_else(|| {
                    ControlError::new(
                        ErrorCode::MissingTarget,
                        format!("{} requires an active terminal session", action.as_str()),
                    )
                }),
            Some(SessionTarget::Id { id }) => pane_group
                .visible_pane_ids()
                .into_iter()
                .find(|pane_id| pane_id.to_string() == id.0)
                .filter(|pane_id| {
                    pane_group
                        .terminal_view_from_pane_id(*pane_id, ctx)
                        .is_some()
                })
                .ok_or_else(|| {
                    ControlError::new(
                        ErrorCode::StaleTarget,
                        format!(
                            "{} cannot resolve the requested session id",
                            action.as_str()
                        ),
                    )
                }),
        })?;
    if target.pane.is_some() {
        let pane_id = target_pane_id(action, target, pane_group, ctx)?;
        if pane_id != session_pane_id {
            return Err(ControlError::new(
                ErrorCode::TargetStateConflict,
                format!(
                    "{} pane and session selectors resolve different targets",
                    action.as_str()
                ),
            ));
        }
    }
    Ok(session_pane_id)
}
fn invalid_params<T>(action: ActionKind) -> Result<T, ControlError> {
    Err(ControlError::new(
        ErrorCode::InvalidParams,
        format!(
            "{} received parameters with the wrong shape",
            action.as_str()
        ),
    ))
}

fn pane_direction(direction: ControlDirection) -> Result<Direction, ControlError> {
    match direction {
        ControlDirection::Left => Ok(Direction::Left),
        ControlDirection::Right => Ok(Direction::Right),
        ControlDirection::Up => Ok(Direction::Up),
        ControlDirection::Down => Ok(Direction::Down),
        ControlDirection::Previous | ControlDirection::Next => Err(ControlError::new(
            ErrorCode::InvalidParams,
            "pane.split only accepts left, right, up, or down",
        )),
    }
}

fn line_and_column(params: &FileOpenParams) -> Result<Option<LineAndColumnArg>, ControlError> {
    let Some(line) = params.line else {
        if params.column.is_some() {
            return Err(ControlError::new(
                ErrorCode::InvalidParams,
                "file.open column requires a line",
            ));
        }
        return Ok(None);
    };
    let line_num = usize::try_from(line).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidParams,
            "file.open line is out of range",
            err.to_string(),
        )
    })?;
    let column_num = params
        .column
        .map(usize::try_from)
        .transpose()
        .map_err(|err| {
            ControlError::with_details(
                ErrorCode::InvalidParams,
                "file.open column is out of range",
                err.to_string(),
            )
        })?;
    Ok(Some(LineAndColumnArg {
        line_num,
        column_num,
    }))
}

fn ack(instance_id: &Option<InstanceId>, action: ActionKind) -> serde_json::Value {
    json!({
        "action": action.as_str(),
        "ok": true,
        "instance_id": instance_id.as_ref().map(|id| id.0.as_str()),
    })
}
