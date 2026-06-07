//! Target and parameter validation for retained local-control actions.
use ::local_control::protocol::{
    ActionNameParams, ActionParameterSpec, BindingNameParams, BooleanValueParams, ColorValueParams,
    DirectionParams, EmptyParams, FileOpenParams, KeyParams, KeyValueParams, LimitParams,
    NamespaceParams, PageQueryParams, QueryParams, RenameParams, ResizeParams, TabActivateParams,
    TabCloseParams, TabCreateParams, TargetSelector, TextParams, ThemeNameParams, WindowTarget,
};
use ::local_control::{ActionKind, ControlError, ErrorCode, TargetScope};
use warpui::{ModelContext, WindowId};

use crate::local_control::handlers::metadata::action_metadata_for_name;
use crate::local_control::LocalControlBridge;

pub(crate) fn validate_tab_create_target(target: &TargetSelector) -> Result<(), ControlError> {
    if target.tab.is_some() || target.pane.is_some() || target.session.is_some() {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "tab.create accepts only a window selector",
        ));
    }
    Ok(())
}

/// Validates action-specific params against the catalog's strict standalone type.
pub(crate) fn validate_action_params(action: &::local_control::Action) -> Result<(), ControlError> {
    if !action.kind.is_implemented() {
        return Ok(());
    }
    match action.kind.metadata().parameter_spec {
        ActionParameterSpec::None => parse_params::<EmptyParams>(action),
        ActionParameterSpec::ActionName => {
            let params = action.params_as::<ActionNameParams>()?;
            action_metadata_for_name(&params.action).map(|_| ())
        }
        ActionParameterSpec::BindingName => parse_params::<BindingNameParams>(action),
        ActionParameterSpec::BooleanValue => parse_params::<BooleanValueParams>(action),
        ActionParameterSpec::ColorValue => parse_params::<ColorValueParams>(action),
        ActionParameterSpec::Direction => parse_params::<DirectionParams>(action),
        ActionParameterSpec::FileOpen => parse_params::<FileOpenParams>(action),
        ActionParameterSpec::Key => parse_params::<KeyParams>(action),
        ActionParameterSpec::KeyValue => parse_params::<KeyValueParams>(action),
        ActionParameterSpec::Limit => parse_params::<LimitParams>(action),
        ActionParameterSpec::Namespace => parse_params::<NamespaceParams>(action),
        ActionParameterSpec::PageQuery => parse_params::<PageQueryParams>(action),
        ActionParameterSpec::Query => parse_params::<QueryParams>(action),
        ActionParameterSpec::Rename => parse_params::<RenameParams>(action),
        ActionParameterSpec::Resize => parse_params::<ResizeParams>(action),
        ActionParameterSpec::TabActivate => parse_params::<TabActivateParams>(action),
        ActionParameterSpec::TabClose => parse_params::<TabCloseParams>(action),
        ActionParameterSpec::TabCreate => parse_params::<TabCreateParams>(action),
        ActionParameterSpec::Text => parse_params::<TextParams>(action),
        ActionParameterSpec::ThemeName => parse_params::<ThemeNameParams>(action),
    }
}

pub(crate) fn validate_action_target(
    action: ActionKind,
    target: &TargetSelector,
) -> Result<(), ControlError> {
    let has_target = target.window.is_some()
        || target.tab.is_some()
        || target.pane.is_some()
        || target.session.is_some();
    let rejects_all_targets = match action.metadata().target_scope {
        TargetScope::Instance => action != ActionKind::AppFocus,
        TargetScope::Appearance
        | TargetScope::Settings
        | TargetScope::Keybinding
        | TargetScope::Action
        | TargetScope::Capability => true,
        TargetScope::Window
        | TargetScope::Tab
        | TargetScope::Pane
        | TargetScope::Session
        | TargetScope::Block
        | TargetScope::Input
        | TargetScope::Surface
        | TargetScope::File => false,
    };
    if rejects_all_targets && has_target {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{} does not accept target selectors", action.as_str()),
        ));
    }
    Ok(())
}

pub(super) fn target_window_id_for_target(
    ctx: &mut ModelContext<LocalControlBridge>,
    target: &TargetSelector,
    action: ActionKind,
) -> Result<WindowId, ControlError> {
    match target.window.as_ref() {
        None | Some(WindowTarget::Active) => active_or_single_window_id(ctx, action),
        Some(WindowTarget::Id { id }) => ctx
            .window_ids()
            .find(|window_id| window_id.to_string() == id.0)
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    format!("{} cannot resolve the requested window id", action.as_str()),
                )
            }),
        Some(WindowTarget::Index { index }) => {
            resolve_index_from_ids(ctx.window_ids(), *index, action)
        }
        Some(WindowTarget::Title { .. }) => Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} only supports active, opaque window id, and window index selectors",
                action.as_str()
            ),
        )),
    }
}

#[cfg(test)]
pub(crate) fn require_active_window_id(
    active_window: Option<WindowId>,
) -> Result<WindowId, ControlError> {
    require_active_window_id_for_action(active_window, ActionKind::TabCreate)
}

pub(crate) fn require_active_window_id_for_action(
    active_window: Option<WindowId>,
    action: ActionKind,
) -> Result<WindowId, ControlError> {
    active_window.ok_or_else(|| {
        ControlError::new(
            ErrorCode::MissingTarget,
            format!("{} requires an active Warp window", action.as_str()),
        )
    })
}

fn active_or_single_window_id(
    ctx: &mut ModelContext<LocalControlBridge>,
    action: ActionKind,
) -> Result<WindowId, ControlError> {
    if let Some(window_id) = ctx.windows().active_window() {
        return Ok(window_id);
    }
    let window_ids = ctx.window_ids().collect::<Vec<_>>();
    match window_ids.as_slice() {
        [window_id] => Ok(*window_id),
        [] => require_active_window_id_for_action(None, action),
        _ => Err(ControlError::new(
            ErrorCode::AmbiguousTarget,
            format!(
                "{} requires an explicit window selector when no Warp window is active",
                action.as_str()
            ),
        )),
    }
}

pub(crate) fn resolve_index_from_ids(
    ids: impl Iterator<Item = WindowId>,
    index: u32,
    action: ActionKind,
) -> Result<WindowId, ControlError> {
    let mut ids = ids.collect::<Vec<_>>();
    ids.sort_by_key(ToString::to_string);
    ids.get(index as usize).copied().ok_or_else(|| {
        ControlError::new(
            ErrorCode::MissingTarget,
            format!(
                "{} cannot resolve the requested window index",
                action.as_str()
            ),
        )
    })
}

#[cfg(test)]
pub(crate) fn resolve_title_from_matches(
    matching: &[WindowId],
    action: ActionKind,
) -> Result<WindowId, ControlError> {
    match matching {
        [window_id] => Ok(*window_id),
        [] => Err(ControlError::new(
            ErrorCode::MissingTarget,
            format!(
                "{} cannot resolve the requested window title",
                action.as_str()
            ),
        )),
        _ => Err(ControlError::new(
            ErrorCode::AmbiguousTarget,
            format!("{} resolved multiple windows by title", action.as_str()),
        )),
    }
}
fn parse_params<T: serde::de::DeserializeOwned>(
    action: &::local_control::Action,
) -> Result<(), ControlError> {
    action.params_as::<T>().map(|_| ())
}
