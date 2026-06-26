use std::collections::HashMap;
use std::sync::Arc;

use uuid::Uuid;
use warpui::{App, SingletonEntity};

use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionResultType,
    AIAgentActionType, AIAgentAttachment, AIAgentContext, AIAgentInput, CancellationReason,
    ImageContext, PassiveSuggestionTrigger, RequestCommandOutputResult, UserQueryMode,
};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::{BlocklistAIHistoryModel, PendingAttachment, PendingFile};
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

fn new_ambient_agent_task_id() -> AmbientAgentTaskId {
    Uuid::new_v4().to_string().parse().unwrap()
}

fn image_attachment(file_name: &str) -> PendingAttachment {
    PendingAttachment::Image(ImageContext {
        data: String::new(),
        mime_type: "image/png".to_owned(),
        file_name: file_name.to_owned(),
        is_figma: false,
    })
}

fn file_attachment(file_name: &str) -> PendingAttachment {
    PendingAttachment::File(PendingFile {
        file_name: file_name.to_owned(),
        file_path: file_name.into(),
        mime_type: "text/plain".to_owned(),
    })
}

#[test]
fn passive_suggestions_request_params_omit_ambient_agent_task_id() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |terminal, ctx| {
            let task_id = new_ambient_agent_task_id();
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_conversation(terminal.id(), false, false, false, ctx)
                });

            terminal.ai_controller().update(ctx, |controller, ctx| {
                controller.set_ambient_agent_task_id(Some(task_id), ctx);

                assert_eq!(controller.get_ambient_agent_task_id(), Some(task_id));
                assert_eq!(
                    controller
                        .build_passive_suggestions_request_params(
                            Some(conversation_id),
                            PassiveSuggestionTrigger::FilesChanged,
                            vec![],
                            ctx,
                        )
                        .expect("existing conversation should build passive suggestion params")
                        .1
                        .ambient_agent_task_id,
                    None
                );
                assert_eq!(
                    controller
                        .build_passive_suggestions_request_params(
                            None,
                            PassiveSuggestionTrigger::FilesChanged,
                            vec![],
                            ctx,
                        )
                        .expect("new conversation should build passive suggestion params")
                        .1
                        .ambient_agent_task_id,
                    None
                );
            });
        });
    });
}

#[test]
fn input_for_query_converts_prompt_attachments_and_ignores_live_staging() {
    // `input_for_query` builds its image/file context purely from the explicitly-provided
    // attachment set (resolved by `send_query` from either the queued row or live staging),
    // never from the context model's pending attachments.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |terminal, ctx| {
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_conversation(terminal.id(), false, false, false, ctx)
                });

            let controller = terminal.ai_controller();
            let context_model = controller.as_ref(ctx).context_model.clone();
            let active_session = controller.as_ref(ctx).active_session.clone();

            // Stage *live* attachments that must NOT leak into a query built from a different,
            // explicitly-provided attachment set.
            context_model.update(ctx, |m, ctx| {
                m.append_pending_attachments(
                    vec![image_attachment("live.png"), file_attachment("live.txt")],
                    ctx,
                );
            });

            let task_id = TaskId::new("test-task".to_owned());
            // Two files sharing a basename to exercise duplicate-basename suffixing.
            let prompt_attachments = vec![
                image_attachment("queued.png"),
                file_attachment("notes.txt"),
                file_attachment("notes.txt"),
            ];

            let input = super::input_for_query(
                "build a query".to_owned(),
                &task_id,
                conversation_id,
                None,
                UserQueryMode::Normal,
                None,
                HashMap::new(),
                prompt_attachments,
                context_model.as_ref(ctx),
                active_session.as_ref(ctx),
                ctx,
            );

            let AIAgentInput::UserQuery {
                context,
                referenced_attachments,
                ..
            } = input
            else {
                panic!("expected UserQuery");
            };

            // The provided image is attached as image context; the live-staged image is not.
            let image_names: Vec<&str> = context
                .iter()
                .filter_map(|c| match c {
                    AIAgentContext::Image(img) => Some(img.file_name.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(image_names, vec!["queued.png"]);

            // The provided files are attached as FilePathReference with duplicate-basename
            // suffixing; the live-staged file is not.
            let mut file_names: Vec<String> = referenced_attachments
                .values()
                .filter_map(|a| match a {
                    AIAgentAttachment::FilePathReference { file_name, .. } => {
                        Some(file_name.clone())
                    }
                    _ => None,
                })
                .collect();
            file_names.sort();
            assert_eq!(
                file_names,
                vec!["notes.txt".to_owned(), "notes.txt".to_owned()]
            );
            assert!(referenced_attachments.contains_key("notes.txt"));
            assert!(referenced_attachments.contains_key("notes.txt (1)"));
            assert!(!referenced_attachments.contains_key("live.txt"));
        });
    });
}

#[test]
fn cancelling_conversation_aborts_pending_auto_resume() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        // An ID with no backing conversation: if the scheduled wait ever
        // completes, the resume is a harmless no-op.
        let conversation_id = AIConversationId::new();

        terminal.update(&mut app, |terminal, ctx| {
            terminal.ai_controller().update(ctx, |controller, ctx| {
                controller.schedule_auto_resume_after_error(conversation_id, ctx);
                assert!(controller
                    .pending_auto_resume_handles
                    .contains_key(&conversation_id));

                controller.cancel_conversation_progress(
                    conversation_id,
                    CancellationReason::ManuallyCancelled,
                    ctx,
                );
                assert!(!controller
                    .pending_auto_resume_handles
                    .contains_key(&conversation_id));
            });
        });
    });
}

/// Regression test for orphaned conversations: when an action's preprocessing
/// completes but enqueues nothing (here, because a result for the action already
/// exists and the dedup guard drops it), the conversation must not be left stuck
/// `InProgress` with no in-flight stream and no pending/running actions. It should
/// resolve to a terminal status (or trigger a follow-up).
#[test]
fn success_with_actions_that_drop_in_preprocessing_does_not_orphan_conversation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let action_id = AIAgentActionId::from("orphan-action".to_owned());
        let task_id = TaskId::new("orphan-task".to_owned());

        let conversation_id = terminal.update(&mut app, |terminal, ctx| {
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_conversation(terminal.id(), false, false, false, ctx)
                });

            let action_model = terminal.ai_controller().as_ref(ctx).action_model.clone();

            // Seed a finished result for `action_id` so the preprocessing dedup guard drops
            // the action when it is queued below, enqueuing nothing executable. A cancelled
            // result keeps resolution offline (it won't trigger a follow-up request).
            action_model.update(ctx, |action_model, _| {
                action_model.insert_finished_action_result_for_test(
                    conversation_id,
                    Arc::new(AIAgentActionResult {
                        id: action_id.clone(),
                        task_id: task_id.clone(),
                        result: AIAgentActionResultType::RequestCommandOutput(
                            RequestCommandOutputResult::CancelledBeforeExecution,
                        ),
                    }),
                );
            });

            // Queue an action sharing that id. Preprocessing is spawned but has not yet run.
            action_model.update(ctx, |action_model, ctx| {
                action_model.queue_actions(
                    vec![AIAgentAction {
                        id: action_id.clone(),
                        task_id: task_id.clone(),
                        action: AIAgentActionType::InitProject,
                        requires_result: true,
                    }],
                    conversation_id,
                    ctx,
                );
            });

            conversation_id
        });

        // Until preprocessing drains, the conversation is still in progress.
        terminal.update(&mut app, |_terminal, ctx| {
            assert!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&conversation_id)
                    .expect("conversation exists")
                    .status()
                    .is_in_progress(),
                "conversation should be InProgress before preprocessing drains"
            );
        });

        // Drive the spawned preprocessing future to completion.
        for _ in 0..5 {
            futures_lite::future::yield_now().await;
        }

        terminal.update(&mut app, |_terminal, ctx| {
            let status = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .expect("conversation exists")
                .status()
                .clone();
            assert!(
                !matches!(status, ConversationStatus::InProgress),
                "conversation whose only action dropped during preprocessing must not remain \
                 InProgress; got {status:?}"
            );
        });
    });
}
