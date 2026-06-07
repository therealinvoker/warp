//! Metadata response builders for local-control introspection actions.
use ::local_control::protocol::{
    ActionNameParams, LimitParams, PaneTarget, SessionTarget, TabTarget, TargetSelector,
    WindowTarget,
};
use ::local_control::{
    Action, ActionKind, ActionMetadata, ControlError, ErrorCode, InstanceId, PROTOCOL_VERSION,
};
use serde::Serialize;
use serde_json::{json, Value};
use warp_core::channel::ChannelState;
use warpui::{ModelContext, ViewHandle, WindowId};

use crate::local_control::resolver::require_active_window_id_for_action;
use crate::local_control::LocalControlBridge;
use crate::pane_group::{PaneGroup, PaneId};
use crate::workspace::Workspace;

#[derive(Serialize)]
struct InstanceResponse<'a> {
    action: &'static str,
    instance_id: Option<&'a str>,
    pid: u32,
    channel: String,
    app_id: String,
    protocol_version: u32,
    actions: Vec<ActionMetadata>,
}

fn active_session_target(target: &TargetSelector) -> TargetSelector {
    if !matches!(target.session, Some(SessionTarget::Active)) {
        return target.clone();
    }
    TargetSelector {
        window: target.window.clone().or(Some(WindowTarget::Active)),
        tab: target.tab.clone().or(Some(TabTarget::Active)),
        pane: target.pane.clone().or(Some(PaneTarget::Active)),
        session: target.session.clone(),
    }
}

fn select_session_entries(
    entries: Vec<SessionEntry>,
    session: Option<&SessionTarget>,
    action: ActionKind,
) -> Result<Vec<SessionEntry>, ControlError> {
    match session {
        None => Ok(entries),
        Some(SessionTarget::Active) => explicit_matches(
            entries
                .into_iter()
                .filter(|entry| entry.is_active)
                .collect(),
            action,
            "active session",
            ErrorCode::MissingTarget,
        ),
        Some(SessionTarget::Id { id }) => explicit_matches(
            entries
                .into_iter()
                .filter(|entry| entry.pane_id.to_string() == id.0)
                .collect(),
            action,
            "session id",
            ErrorCode::StaleTarget,
        ),
    }
}

#[derive(Serialize)]
struct PingResponse<'a> {
    action: &'static str,
    ok: bool,
    instance_id: Option<&'a str>,
    protocol_version: u32,
}

#[derive(Serialize)]
struct VersionResponse<'a> {
    action: &'static str,
    instance_id: Option<&'a str>,
    protocol_version: u32,
    channel: String,
    app_id: String,
}

#[derive(Serialize)]
struct ActiveTargetChain {
    instance_id: Option<String>,
    window_id: Option<String>,
    tab_id: Option<String>,
    pane_id: Option<String>,
    session_id: Option<String>,
}

#[derive(Clone, Copy)]
struct WindowEntry {
    window_id: WindowId,
    index: usize,
}

#[derive(Clone)]
struct TabEntry {
    window_id: WindowId,
    window_index: usize,
    index: usize,
    workspace_active_tab_index: usize,
    pane_group: ViewHandle<PaneGroup>,
}

#[derive(Clone)]
struct PaneEntry {
    window_id: WindowId,
    window_index: usize,
    tab_id: String,
    tab_index: usize,
    index: usize,
    pane_group: ViewHandle<PaneGroup>,
    pane_id: PaneId,
}

struct SessionEntry {
    window_id: WindowId,
    window_index: usize,
    tab_id: String,
    tab_index: usize,
    pane_id: PaneId,
    pane_index: usize,
    is_active: bool,
}

pub(crate) fn instance(
    instance_id: &Option<InstanceId>,
) -> Result<serde_json::Value, ControlError> {
    to_json_value(InstanceResponse {
        action: ActionKind::InstanceList.as_str(),
        instance_id: instance_id.as_ref().map(|id| id.0.as_str()),
        pid: std::process::id(),
        channel: ChannelState::channel().to_string(),
        app_id: ChannelState::app_id().to_string(),
        protocol_version: PROTOCOL_VERSION,
        actions: ActionKind::implemented_metadata(),
    })
}

pub(crate) fn ping(instance_id: &Option<InstanceId>) -> Result<serde_json::Value, ControlError> {
    to_json_value(PingResponse {
        action: ActionKind::AppPing.as_str(),
        ok: true,
        instance_id: instance_id.as_ref().map(|id| id.0.as_str()),
        protocol_version: PROTOCOL_VERSION,
    })
}

pub(crate) fn version(instance_id: &Option<InstanceId>) -> Result<serde_json::Value, ControlError> {
    to_json_value(VersionResponse {
        action: ActionKind::AppVersion.as_str(),
        instance_id: instance_id.as_ref().map(|id| id.0.as_str()),
        protocol_version: PROTOCOL_VERSION,
        channel: ChannelState::channel().to_string(),
        app_id: ChannelState::app_id().to_string(),
    })
}

pub(crate) fn active(
    instance_id: &Option<InstanceId>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    Ok(json!({
        "action": ActionKind::AppActive.as_str(),
        "active": active_chain(instance_id, ctx)?,
    }))
}

pub(crate) fn inspect(
    instance_id: &Option<InstanceId>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    Ok(json!({
        "action": ActionKind::InstanceInspect.as_str(),
        "instance_id": instance_id.as_ref().map(|id| id.0.as_str()),
        "pid": std::process::id(),
        "channel": ChannelState::channel().to_string(),
        "app_id": ChannelState::app_id().to_string(),
        "protocol_version": PROTOCOL_VERSION,
        "active": active_chain(instance_id, ctx)?,
        "actions": ActionKind::implemented_metadata(),
    }))
}

pub(crate) fn action_list() -> serde_json::Value {
    json!({
        "action": ActionKind::ActionList.as_str(),
        "actions": ActionKind::implemented_metadata(),
    })
}

pub(crate) fn action_inspect(action: &Action) -> Result<serde_json::Value, ControlError> {
    let params = action_name_params(action)?;
    let metadata = action_metadata_for_name(&params.action)?;
    Ok(json!({
        "action": ActionKind::ActionInspect.as_str(),
        "metadata": metadata,
    }))
}

pub(crate) fn capability_list() -> serde_json::Value {
    json!({
        "action": ActionKind::CapabilityList.as_str(),
        "capabilities": ActionKind::implemented_metadata(),
    })
}

pub(crate) fn capability_inspect(action: &Action) -> Result<serde_json::Value, ControlError> {
    let params = action_name_params(action)?;
    let metadata = action_metadata_for_name(&params.action)?;
    Ok(json!({
        "action": ActionKind::CapabilityInspect.as_str(),
        "capability": metadata,
    }))
}

pub(crate) fn window_list(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        ActionKind::WindowList,
        target.tab.is_some() || target.pane.is_some() || target.session.is_some(),
        "tab, pane, or session selectors",
    )?;
    let active_window = ctx.windows().active_window();
    let mut windows = Vec::new();
    for entry in select_window_entries(target, false, ActionKind::WindowList, ctx)? {
        windows.push(json!({
            "window_id": entry.window_id.to_string(),
            "index": entry.index as u32,
            "is_active": Some(entry.window_id) == active_window,
            "has_workspace": workspace_for_window(entry.window_id, ActionKind::WindowList, ctx)?.is_some(),
        }));
    }
    Ok(json!({
        "action": ActionKind::WindowList.as_str(),
        "windows": windows,
    }))
}

pub(crate) fn window_inspect(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        ActionKind::WindowInspect,
        target.tab.is_some() || target.pane.is_some() || target.session.is_some(),
        "tab, pane, or session selectors",
    )?;
    let target = TargetSelector {
        window: target.window.clone().or(Some(WindowTarget::Active)),
        tab: None,
        pane: None,
        session: None,
    };
    let data = window_list(&target, ctx)?;
    let window = single_entry(data.get("windows"), ActionKind::WindowInspect)?;
    Ok(json!({
        "action": ActionKind::WindowInspect.as_str(),
        "window": window,
    }))
}

pub(crate) fn tab_list(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        ActionKind::TabList,
        target.pane.is_some() || target.session.is_some(),
        "pane or session selectors",
    )?;
    let entries = select_tab_entries(target, ActionKind::TabList, ctx)?;
    let tabs = entries
        .into_iter()
        .map(|entry| {
            json!({
                "tab_id": entry.pane_group.id().to_string(),
                "window_id": entry.window_id.to_string(),
                "window_index": entry.window_index as u32,
                "index": entry.index as u32,
                "is_active": entry.index == entry.workspace_active_tab_index,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "action": ActionKind::TabList.as_str(),
        "tabs": tabs,
    }))
}

pub(crate) fn tab_inspect(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        ActionKind::TabInspect,
        target.pane.is_some() || target.session.is_some(),
        "pane or session selectors",
    )?;
    let target = TargetSelector {
        window: target.window.clone(),
        tab: target.tab.clone().or(Some(TabTarget::Active)),
        pane: None,
        session: None,
    };
    let data = tab_list(&target, ctx)?;
    let tab = single_entry(data.get("tabs"), ActionKind::TabInspect)?;
    Ok(json!({
        "action": ActionKind::TabInspect.as_str(),
        "tab": tab,
    }))
}

pub(crate) fn pane_list(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        ActionKind::PaneList,
        target.session.is_some(),
        "session selectors",
    )?;
    let entries = select_pane_entries(target, ActionKind::PaneList, ctx)?;
    let mut panes = Vec::new();
    for entry in entries {
        let (is_active, has_terminal_session) = entry.pane_group.read(ctx, |pane_group, ctx| {
            (
                pane_group.focused_pane_id(ctx) == entry.pane_id,
                pane_group
                    .terminal_view_from_pane_id(entry.pane_id, ctx)
                    .is_some(),
            )
        });
        panes.push(json!({
            "pane_id": entry.pane_id.to_string(),
            "tab_id": entry.tab_id,
            "tab_index": entry.tab_index as u32,
            "window_id": entry.window_id.to_string(),
            "window_index": entry.window_index as u32,
            "index": entry.index as u32,
            "is_active": is_active,
            "has_terminal_session": has_terminal_session,
        }));
    }
    Ok(json!({
        "action": ActionKind::PaneList.as_str(),
        "panes": panes,
    }))
}

pub(crate) fn pane_inspect(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        ActionKind::PaneInspect,
        target.session.is_some(),
        "session selectors",
    )?;
    let target = TargetSelector {
        window: target.window.clone(),
        tab: target.tab.clone(),
        pane: target.pane.clone().or(Some(PaneTarget::Active)),
        session: None,
    };
    let data = pane_list(&target, ctx)?;
    let pane = single_entry(data.get("panes"), ActionKind::PaneInspect)?;
    Ok(json!({
        "action": ActionKind::PaneInspect.as_str(),
        "pane": pane,
    }))
}

pub(crate) fn session_list(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let target = active_session_target(target);
    let pane_entries = select_pane_entries(&target, ActionKind::SessionList, ctx)?;
    let entries = select_session_entries(
        session_entries_for_panes(pane_entries, ctx),
        target.session.as_ref(),
        ActionKind::SessionList,
    )?;
    let sessions = session_values(entries);
    Ok(json!({
        "action": ActionKind::SessionList.as_str(),
        "sessions": sessions,
    }))
}

pub(crate) fn session_inspect(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let target = active_session_target(target);
    let pane_entries = select_pane_entries(&target, ActionKind::SessionInspect, ctx)?;
    let entries = select_session_entries(
        session_entries_for_panes(pane_entries, ctx),
        target.session.as_ref().or(Some(&SessionTarget::Active)),
        ActionKind::SessionInspect,
    )?;
    let data = json!({ "sessions": session_values(entries) });
    let session = single_entry(data.get("sessions"), ActionKind::SessionInspect)?;
    Ok(json!({
        "action": ActionKind::SessionInspect.as_str(),
        "session": session,
    }))
}

pub(crate) fn block_list(
    action: &Action,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let LimitParams { limit } = action.params_as()?;
    let target = TargetSelector {
        window: target.window.clone(),
        tab: target.tab.clone(),
        pane: target.pane.clone(),
        session: target.session.clone().or(Some(SessionTarget::Active)),
    };
    let target = active_session_target(&target);
    let pane_entries = select_pane_entries(&target, ActionKind::BlockList, ctx)?;
    select_session_entries(
        session_entries_for_panes(pane_entries.clone(), ctx),
        target.session.as_ref(),
        ActionKind::BlockList,
    )?;
    let mut blocks = Vec::new();
    for entry in pane_entries {
        let is_selected_session = entry.pane_group.read(ctx, |pane_group, ctx| {
            let has_terminal = pane_group
                .terminal_view_from_pane_id(entry.pane_id, ctx)
                .is_some();
            has_terminal
                && match target.session.as_ref() {
                    None => true,
                    Some(SessionTarget::Active) => {
                        pane_group.active_session_id(ctx).map(PaneId::from) == Some(entry.pane_id)
                    }
                    Some(SessionTarget::Id { id }) => entry.pane_id.to_string() == id.0,
                }
        });
        if !is_selected_session {
            continue;
        }
        let terminal_view = entry.pane_group.read(ctx, |pane_group, ctx| {
            pane_group.terminal_view_from_pane_id(entry.pane_id, ctx)
        });
        let Some(terminal_view) = terminal_view else {
            continue;
        };
        let mut session_blocks = terminal_view.read(ctx, |terminal_view, _| {
            let model = terminal_view.model.lock();
            model
                .block_list()
                .blocks()
                .iter()
                .enumerate()
                .map(|(index, block)| {
                    json!({
                        "block_id": block.id().to_string(),
                        "session_id": entry.pane_id.to_string(),
                        "index": index as u32,
                    })
                })
                .collect::<Vec<_>>()
        });
        if let Some(limit) = limit {
            let keep_from = session_blocks.len().saturating_sub(limit as usize);
            session_blocks.drain(0..keep_from);
        }
        blocks.extend(session_blocks);
    }
    Ok(json!({
        "action": ActionKind::BlockList.as_str(),
        "blocks": blocks,
    }))
}
pub(crate) fn action_metadata_for_name(action_name: &str) -> Result<ActionMetadata, ControlError> {
    ActionKind::implemented_metadata()
        .into_iter()
        .find(|metadata| metadata.name == action_name)
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::NotAllowlisted,
                "requested action is not an implemented local-control action",
            )
        })
}

fn action_name_params(action: &Action) -> Result<ActionNameParams, ControlError> {
    action.params_as()
}

fn to_json_value<T: Serialize>(response: T) -> Result<serde_json::Value, ControlError> {
    serde_json::to_value(response).map_err(|error| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to serialize local-control metadata response",
            error.to_string(),
        )
    })
}

fn active_chain(
    instance_id: &Option<InstanceId>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ActiveTargetChain, ControlError> {
    let instance_id = instance_id.as_ref().map(|id| id.0.clone());
    let Some(window_id) = ctx.windows().active_window() else {
        return Ok(ActiveTargetChain {
            instance_id,
            window_id: None,
            tab_id: None,
            pane_id: None,
            session_id: None,
        });
    };
    let window_id_string = window_id.to_string();
    let Some(workspace) = workspace_for_window(window_id, ActionKind::AppActive, ctx)? else {
        return Ok(ActiveTargetChain {
            instance_id,
            window_id: Some(window_id_string),
            tab_id: None,
            pane_id: None,
            session_id: None,
        });
    };
    let (tab_id, pane_id, session_id) = workspace.read(ctx, |workspace, ctx| {
        let pane_group = workspace.active_tab_pane_group();
        let pane_group_ref = pane_group.as_ref(ctx);
        (
            Some(pane_group.id().to_string()),
            Some(pane_group_ref.focused_pane_id(ctx).to_string()),
            pane_group_ref
                .active_session_id(ctx)
                .map(|session_id| PaneId::from(session_id).to_string()),
        )
    });
    Ok(ActiveTargetChain {
        instance_id,
        window_id: Some(window_id_string),
        tab_id,
        pane_id,
        session_id,
    })
}

fn select_window_entries(
    target: &TargetSelector,
    force_active_default: bool,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Vec<WindowEntry>, ControlError> {
    let entries = window_entries(ctx);
    match target.window.as_ref() {
        None if force_active_default => {
            let active =
                require_active_window_id_for_action(ctx.windows().active_window(), action)?;
            explicit_matches(
                entries
                    .into_iter()
                    .filter(|entry| entry.window_id == active)
                    .collect(),
                action,
                "active window",
                ErrorCode::MissingTarget,
            )
        }
        None => Ok(entries),
        Some(WindowTarget::Active) => {
            let active =
                require_active_window_id_for_action(ctx.windows().active_window(), action)?;
            explicit_matches(
                entries
                    .into_iter()
                    .filter(|entry| entry.window_id == active)
                    .collect(),
                action,
                "active window",
                ErrorCode::MissingTarget,
            )
        }
        Some(WindowTarget::Id { id }) => explicit_matches(
            entries
                .into_iter()
                .filter(|entry| entry.window_id.to_string() == id.0)
                .collect(),
            action,
            "window id",
            ErrorCode::StaleTarget,
        ),
        Some(WindowTarget::Index { index }) => explicit_matches(
            entries
                .into_iter()
                .filter(|entry| entry.index as u32 == *index)
                .collect(),
            action,
            "window index",
            ErrorCode::StaleTarget,
        ),
        Some(WindowTarget::Title { .. }) => Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} only supports active, opaque window id, and window index selectors",
                action.as_str()
            ),
        )),
    }
}

fn select_tab_entries(
    target: &TargetSelector,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Vec<TabEntry>, ControlError> {
    let force_active_window = matches!(
        target.tab,
        Some(TabTarget::Active | TabTarget::Index { .. })
    ) || matches!(
        target.pane,
        Some(PaneTarget::Active | PaneTarget::Index { .. })
    );
    let windows = select_window_entries(target, force_active_window, action, ctx)?;
    let entries = tab_entries_for_windows(windows, action, ctx)?;
    let requires_active_tab_default = matches!(
        target.pane,
        Some(PaneTarget::Active | PaneTarget::Index { .. })
    );
    match target.tab.as_ref() {
        None if requires_active_tab_default => explicit_matches(
            entries
                .into_iter()
                .filter(|entry| entry.index == entry.workspace_active_tab_index)
                .collect(),
            action,
            "active tab",
            ErrorCode::MissingTarget,
        ),
        None => Ok(entries),
        Some(TabTarget::Active) => explicit_matches(
            entries
                .into_iter()
                .filter(|entry| entry.index == entry.workspace_active_tab_index)
                .collect(),
            action,
            "active tab",
            ErrorCode::MissingTarget,
        ),
        Some(TabTarget::Id { id }) => explicit_matches(
            entries
                .into_iter()
                .filter(|entry| entry.pane_group.id().to_string() == id.0)
                .collect(),
            action,
            "tab id",
            ErrorCode::StaleTarget,
        ),
        Some(TabTarget::Index { index }) => explicit_matches(
            entries
                .into_iter()
                .filter(|entry| entry.index as u32 == *index)
                .collect(),
            action,
            "tab index",
            ErrorCode::StaleTarget,
        ),
        Some(TabTarget::Title { .. }) => Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} only supports active, opaque tab id, and tab index selectors",
                action.as_str()
            ),
        )),
    }
}

fn select_pane_entries(
    target: &TargetSelector,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Vec<PaneEntry>, ControlError> {
    let tabs = select_tab_entries(target, action, ctx)?;
    let entries = pane_entries_for_tabs(tabs, ctx);
    match target.pane.as_ref() {
        None => Ok(entries),
        Some(PaneTarget::Active) => explicit_matches(
            entries
                .into_iter()
                .filter(|entry| {
                    entry.pane_group.read(ctx, |pane_group, ctx| {
                        pane_group.focused_pane_id(ctx) == entry.pane_id
                    })
                })
                .collect(),
            action,
            "active pane",
            ErrorCode::MissingTarget,
        ),
        Some(PaneTarget::Id { id }) => explicit_matches(
            entries
                .into_iter()
                .filter(|entry| entry.pane_id.to_string() == id.0)
                .collect(),
            action,
            "pane id",
            ErrorCode::StaleTarget,
        ),
        Some(PaneTarget::Index { index }) => explicit_matches(
            entries
                .into_iter()
                .filter(|entry| entry.index as u32 == *index)
                .collect(),
            action,
            "pane index",
            ErrorCode::StaleTarget,
        ),
    }
}

fn window_entries(ctx: &mut ModelContext<LocalControlBridge>) -> Vec<WindowEntry> {
    let mut ids = ctx.window_ids().collect::<Vec<_>>();
    ids.sort_by_key(ToString::to_string);
    ids.into_iter()
        .enumerate()
        .map(|(index, window_id)| WindowEntry { window_id, index })
        .collect()
}

fn tab_entries_for_windows(
    windows: Vec<WindowEntry>,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Vec<TabEntry>, ControlError> {
    let mut entries = Vec::new();
    for window in windows {
        let Some(workspace) = workspace_for_window(window.window_id, action, ctx)? else {
            continue;
        };
        entries.extend(workspace.read(ctx, |workspace, _| {
            workspace
                .tab_views()
                .enumerate()
                .map(|(index, pane_group)| TabEntry {
                    window_id: window.window_id,
                    window_index: window.index,
                    index,
                    workspace_active_tab_index: workspace.active_tab_index(),
                    pane_group: pane_group.clone(),
                })
                .collect::<Vec<_>>()
        }));
    }
    Ok(entries)
}

fn pane_entries_for_tabs(
    tabs: Vec<TabEntry>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Vec<PaneEntry> {
    let mut entries = Vec::new();
    for tab in tabs {
        let tab_id = tab.pane_group.id().to_string();
        let pane_group = tab.pane_group.clone();
        let pane_ids = tab
            .pane_group
            .read(ctx, |pane_group, _| pane_group.visible_pane_ids());
        entries.extend(
            pane_ids
                .into_iter()
                .enumerate()
                .map(|(index, pane_id)| PaneEntry {
                    window_id: tab.window_id,
                    window_index: tab.window_index,
                    tab_id: tab_id.clone(),
                    tab_index: tab.index,
                    index,
                    pane_group: pane_group.clone(),
                    pane_id,
                }),
        );
    }
    entries
}

fn session_entries_for_panes(
    panes: Vec<PaneEntry>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Vec<SessionEntry> {
    let mut entries = Vec::new();
    for pane in panes {
        let (has_terminal_session, is_active) = pane.pane_group.read(ctx, |pane_group, ctx| {
            (
                pane_group
                    .terminal_view_from_pane_id(pane.pane_id, ctx)
                    .is_some(),
                pane_group.active_session_id(ctx).map(PaneId::from) == Some(pane.pane_id),
            )
        });
        if has_terminal_session {
            entries.push(SessionEntry {
                window_id: pane.window_id,
                window_index: pane.window_index,
                tab_id: pane.tab_id,
                tab_index: pane.tab_index,
                pane_id: pane.pane_id,
                pane_index: pane.index,
                is_active,
            });
        }
    }
    entries
}

fn session_values(entries: Vec<SessionEntry>) -> Vec<Value> {
    entries
        .into_iter()
        .map(|entry| {
            json!({
                "session_id": entry.pane_id.to_string(),
                "pane_id": entry.pane_id.to_string(),
                "pane_index": entry.pane_index as u32,
                "tab_id": entry.tab_id,
                "tab_index": entry.tab_index as u32,
                "window_id": entry.window_id.to_string(),
                "window_index": entry.window_index as u32,
                "is_active": entry.is_active,
            })
        })
        .collect()
}

fn workspace_for_window(
    window_id: WindowId,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Option<ViewHandle<Workspace>>, ControlError> {
    match ctx.views_of_type::<Workspace>(window_id) {
        None => Ok(None),
        Some(workspaces) => match workspaces.as_slice() {
            [] => Ok(None),
            [workspace] => Ok(Some(workspace.clone())),
            _ => Err(ControlError::new(
                ErrorCode::AmbiguousTarget,
                format!(
                    "{} resolved multiple workspaces in one window",
                    action.as_str()
                ),
            )),
        },
    }
}

fn explicit_matches<T>(
    matches: Vec<T>,
    action: ActionKind,
    selector: &str,
    missing_code: ErrorCode,
) -> Result<Vec<T>, ControlError> {
    match matches.len() {
        0 => Err(ControlError::new(
            missing_code,
            format!(
                "{} cannot resolve the requested {selector}",
                action.as_str()
            ),
        )),
        1 => Ok(matches),
        _ => Err(ControlError::new(
            ErrorCode::AmbiguousTarget,
            format!(
                "{} resolved multiple targets by {selector}",
                action.as_str()
            ),
        )),
    }
}

fn single_entry(value: Option<&Value>, action: ActionKind) -> Result<Value, ControlError> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Err(ControlError::new(
            ErrorCode::Internal,
            format!("{} handler returned malformed metadata", action.as_str()),
        ));
    };
    match items.as_slice() {
        [item] => Ok(item.clone()),
        [] => Err(ControlError::new(
            ErrorCode::MissingTarget,
            format!("{} could not resolve a target", action.as_str()),
        )),
        _ => Err(ControlError::new(
            ErrorCode::AmbiguousTarget,
            format!("{} resolved multiple targets", action.as_str()),
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
