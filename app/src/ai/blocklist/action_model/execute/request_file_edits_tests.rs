use std::cell::{Cell, RefCell};
use std::rc::Rc;

use ai::diff_validation::DiffType;
use async_channel::unbounded;
use warp_files::FileModel;
use warpui::{App, EntityId};

use super::*;
use crate::ai::agent::task::TaskId;
use crate::terminal::model::session::Sessions;
use crate::terminal::model_events::ModelEventDispatcher;

/// A claim source whose backing surface has vanished.
struct GoneSource;

impl PendingEditsSource for GoneSource {
    fn take_edits(&self, _app: &AppContext) -> Option<ClaimedEdits> {
        None
    }
}

/// A claim source that hands out prebuilt edits and records the pull.
struct RecordingSource {
    taken: Rc<Cell<bool>>,
    edits: RefCell<Option<ClaimedEdits>>,
}

impl PendingEditsSource for RecordingSource {
    fn take_edits(&self, _app: &AppContext) -> Option<ClaimedEdits> {
        self.taken.set(true);
        self.edits.borrow_mut().take()
    }
}

/// Builds an executor over a minimal test session.
fn add_executor(app: &mut App) -> ModelHandle<RequestFileEditsExecutor> {
    let sessions = app.add_model(|_| Sessions::new_for_test());
    let (_, model_events_rx) = unbounded();
    let dispatcher =
        app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
    let active_session =
        app.add_model(|ctx| ActiveSession::new(sessions.clone(), dispatcher.clone(), ctx));
    app.add_model(|ctx| RequestFileEditsExecutor::new(active_session, EntityId::new(), ctx))
}

/// Registers the singletons `execute`'s persist path reads.
fn add_execute_singletons(app: &mut App) {
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    app.add_singleton_model(FileModel::new);
    app.add_singleton_model(PersistDiffModel::new);
}

/// Builds a prepared diff creating `/tmp/x.rs`.
fn test_diff() -> FileDiff {
    FileDiff::new(
        String::new(),
        "/tmp/x.rs".to_owned(),
        DiffType::creation("fn main() {}\n".to_owned()),
    )
}

/// Inserts an `Unclaimed` entry for `action_id`.
fn insert_unclaimed(
    app: &mut App,
    executor: &ModelHandle<RequestFileEditsExecutor>,
    action_id: &AIAgentActionId,
) {
    executor.update(app, |executor, _| {
        executor.pending_file_edits.insert(
            action_id.clone(),
            PendingFileEdits::Unclaimed {
                diffs: vec![test_diff()],
                session_type: DiffSessionType::Local,
            },
        );
    });
}

/// Builds a `RequestFileEdits` action with the given id.
fn edit_action(id: &AIAgentActionId) -> AIAgentAction {
    AIAgentAction {
        id: id.clone(),
        task_id: TaskId::new("task".to_owned()),
        action: AIAgentActionType::RequestFileEdits {
            file_edits: Vec::new(),
            title: None,
        },
        requires_result: true,
    }
}

#[test]
fn claim_transfers_ownership_once() {
    App::test((), |mut app| async move {
        let executor = add_executor(&mut app);
        let action_id = AIAgentActionId::from("edit-1".to_owned());
        insert_unclaimed(&mut app, &executor, &action_id);

        let claimed = executor.update(&mut app, |executor, _| {
            executor.claim_prepared_edits(&action_id, Box::new(GoneSource))
        });
        let (diffs, session_type) = claimed.expect("first claim should hand out the diffs");
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].file_path(), "/tmp/x.rs");
        assert!(matches!(session_type, DiffSessionType::Local));

        // The data moved to the first claimer; a second claim gets nothing.
        let reclaimed = executor.update(&mut app, |executor, _| {
            executor.claim_prepared_edits(&action_id, Box::new(GoneSource))
        });
        assert!(reclaimed.is_none());
    });
}

#[test]
fn discard_pending_drops_state_in_any_state() {
    App::test((), |mut app| async move {
        let executor = add_executor(&mut app);

        // Unclaimed entry (e.g. rejected before any surface claimed).
        let unclaimed_id = AIAgentActionId::from("edit-unclaimed".to_owned());
        insert_unclaimed(&mut app, &executor, &unclaimed_id);
        executor.update(&mut app, |executor, _| {
            executor.discard_pending(&unclaimed_id);
        });
        let claim_after_discard = executor.update(&mut app, |executor, _| {
            executor.claim_prepared_edits(&unclaimed_id, Box::new(GoneSource))
        });
        assert!(claim_after_discard.is_none());

        // Claimed entry (rejected after the review surface claimed).
        let claimed_id = AIAgentActionId::from("edit-claimed".to_owned());
        insert_unclaimed(&mut app, &executor, &claimed_id);
        executor.update(&mut app, |executor, _| {
            executor.claim_prepared_edits(&claimed_id, Box::new(GoneSource));
            executor.discard_pending(&claimed_id);
            assert!(!executor.pending_file_edits.contains_key(&claimed_id));
        });
    });
}

#[test]
fn execute_with_vanished_source_fails_recoverably() {
    App::test((), |mut app| async move {
        let executor = add_executor(&mut app);
        let action_id = AIAgentActionId::from("edit-1".to_owned());
        executor.update(&mut app, |executor, _| {
            executor.pending_file_edits.insert(
                action_id.clone(),
                PendingFileEdits::Claimed(Box::new(GoneSource)),
            );
        });

        let action = edit_action(&action_id);
        let conversation_id = AIConversationId::new();
        let execution: AnyActionExecution = executor.update(&mut app, |executor, ctx| {
            executor
                .execute(
                    ExecuteActionInput {
                        action: &action,
                        conversation_id,
                    },
                    ctx,
                )
                .into()
        });

        assert!(matches!(
            execution,
            AnyActionExecution::Sync(AIAgentActionResultType::RequestFileEdits(
                RequestFileEditsResult::DiffApplicationFailed { .. }
            ))
        ));
    });
}

#[test]
fn execute_pulls_claimed_edits_from_the_source() {
    App::test((), |mut app| async move {
        add_execute_singletons(&mut app);
        let executor = add_executor(&mut app);
        let action_id = AIAgentActionId::from("edit-1".to_owned());

        let taken = Rc::new(Cell::new(false));
        let source = RecordingSource {
            taken: taken.clone(),
            edits: RefCell::new(Some(ClaimedEdits {
                edits: vec![ClaimedEdit {
                    diff: test_diff(),
                    final_content: Some("user edited\n".to_owned()),
                    was_edited: true,
                }],
                session_type: DiffSessionType::Local,
            })),
        };
        executor.update(&mut app, |executor, _| {
            executor.pending_file_edits.insert(
                action_id.clone(),
                PendingFileEdits::Claimed(Box::new(source)),
            );
        });

        let action = edit_action(&action_id);
        let conversation_id = AIConversationId::new();
        let execution: AnyActionExecution = executor.update(&mut app, |executor, ctx| {
            executor
                .execute(
                    ExecuteActionInput {
                        action: &action,
                        conversation_id,
                    },
                    ctx,
                )
                .into()
        });

        assert!(taken.get(), "execute should pull the edits from the source");
        assert!(matches!(execution, AnyActionExecution::Async { .. }));
        // The entry is consumed either way.
        executor.update(&mut app, |executor, _| {
            assert!(!executor.pending_file_edits.contains_key(&action_id));
        });
    });
}

#[test]
fn execute_falls_back_to_unclaimed_deltas() {
    App::test((), |mut app| async move {
        add_execute_singletons(&mut app);
        let executor = add_executor(&mut app);
        let action_id = AIAgentActionId::from("edit-1".to_owned());
        insert_unclaimed(&mut app, &executor, &action_id);

        let action = edit_action(&action_id);
        let conversation_id = AIConversationId::new();
        let execution: AnyActionExecution = executor.update(&mut app, |executor, ctx| {
            executor
                .execute(
                    ExecuteActionInput {
                        action: &action,
                        conversation_id,
                    },
                    ctx,
                )
                .into()
        });

        // Nobody claimed (e.g. autoexec beat the DiffsPrepared subscriber):
        // persistence proceeds from the unreviewed deltas.
        assert!(matches!(execution, AnyActionExecution::Async { .. }));
    });
}
