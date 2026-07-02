//! Shared fixtures for `warp_tui` unit tests.
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    ActiveSession, BlocklistAIActionModel, GetRelevantFilesController, ModelEventDispatcher,
    Sessions, TerminalModel,
};
use warpui::{App, EntityId, ModelHandle};

/// Builds a real `BlocklistAIActionModel` over minimal test session state,
/// mirroring what production surfaces inject into transcript views and agent
/// blocks.
pub(crate) fn add_test_action_model(app: &mut App) -> ModelHandle<BlocklistAIActionModel> {
    let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    let sessions = app.add_model(|_| Sessions::new_for_test());
    let (_tx, model_events_rx) = async_channel::unbounded();
    let dispatcher =
        app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
    let active_session =
        app.add_model(|ctx| ActiveSession::new(sessions.clone(), dispatcher.clone(), ctx));
    let get_relevant_files = app.add_model(GetRelevantFilesController::new);
    app.add_model(|ctx| {
        BlocklistAIActionModel::new(
            terminal_model,
            active_session,
            &dispatcher,
            get_relevant_files,
            EntityId::new(),
            ctx,
        )
    })
}
