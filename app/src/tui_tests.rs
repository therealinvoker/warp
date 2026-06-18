//! Headless tests for the TUI root view and core model bootstrap.

use std::time::Duration;

use settings::{PrivatePreferences, PublicPreferences};
use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
use warpui::r#async::Timer;
use warpui::{App, EntityId, SingletonEntity, WindowId};
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::platform::WindowStyle;
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::AddWindowOptions;
use warpui_extras::user_preferences::in_memory::InMemoryPreferences;

use super::input_view::InputAction;
use super::{CoreTuiModel, RootTuiView, INPUT_ROWS};
use crate::ai::blocklist::{AgentSessionOwnerId, BlocklistAIHistoryModel, BlocklistAIPermissions};
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::llms::LLMPreferences;
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::default_terminal::DefaultTerminal;
use crate::interval_timer::IntervalTimer;
use crate::network::NetworkStatus;
use crate::server::server_api::ServerApiProvider;
use crate::workspace::ActiveSession;
use crate::{initialize_app, LaunchMode};

fn window_options() -> AddWindowOptions {
    AddWindowOptions {
        window_style: WindowStyle::NotStealFocus,
        ..Default::default()
    }
}

/// Types `text` into the input one scalar at a time.
fn type_text(app: &App, window_id: WindowId, input_id: EntityId, text: &str) {
    for ch in text.chars() {
        app.dispatch_typed_action(window_id, &[input_id], &InputAction::Insert(ch.to_string()));
    }
}

/// Presses Enter on the input, as its `enter` key binding does.
fn submit(app: &App, window_id: WindowId, input_id: EntityId) {
    app.dispatch_typed_action(window_id, &[input_id], &InputAction::Submit);
}

/// The first row index whose painted line contains `needle`.
fn row_with(lines: &[String], needle: &str) -> Option<usize> {
    lines.iter().position(|line| line.contains(needle))
}

/// Registers the singletons normally installed before `initialize_app`.
fn register_pre_initialize_singletons(app: &mut App) {
    app.add_singleton_model(|ctx| AppExecutionMode::new(ExecutionMode::App, false, ctx));
    app.add_singleton_model(|_| PublicPreferences::new(Box::<InMemoryPreferences>::default()));
    app.add_singleton_model(|_| PrivatePreferences::new(Box::<InMemoryPreferences>::default()));
}

/// Runs the TUI launch-mode singleton bootstrap without creating a TUI window.
fn initialize_tui_app(app: &mut App) {
    register_pre_initialize_singletons(app);
    app.update(|ctx| {
        let _ = initialize_app(
            &LaunchMode::Tui,
            IntervalTimer::new(),
            None,
            ctx,
            Vec::<anyhow::Error>::new(),
        );
    });
}

/// Returns a clone of the in-flight request params for assertions.
fn in_flight_request_params(app: &App) -> crate::ai::agent::api::RequestParams {
    let stream = CoreTuiModel::handle(app).read(app, |model, _| {
        model
            .in_flight_response_stream_for_test()
            .expect("request should be in flight")
    });
    stream.read(app, |stream, _| stream.params_for_test().clone())
}

/// Waits for the single TUI request to finish.
async fn wait_for_tui_request_to_finish(app: &mut App) {
    for _ in 0..600 {
        let has_in_flight =
            CoreTuiModel::handle(app).read(app, |model, _| model.has_in_flight_request());
        if !has_in_flight {
            return;
        }
        Timer::after(Duration::from_millis(250)).await;
    }
    panic!("timed out waiting for TUI request to finish");
}

#[test]
fn submitting_moves_text_into_the_transcript_and_clears_the_focused_input() {
    App::test((), |mut app| async move {
        let (window_id, root) =
            app.update(|ctx| ctx.add_tui_window(window_options(), RootTuiView::new));
        let input_id = app.read(|ctx| root.read(ctx, |view, _| view.input.id()));

        // The input is focused at construction, so the cursor is owned by it.
        assert_eq!(app.focused_view_id(window_id), Some(input_id));

        let mut presenter = TuiPresenter::new();
        // 8 rows tall: the input frame is the bottom `INPUT_ROWS` (3) rows, so
        // the inner text row sits at `height - 2` and the transcript fills the
        // rows above it.
        let width = 40;
        let height = 8;
        let area = TuiRect::new(0, 0, width, height);
        let input_text_row = height - 2;
        let first_input_row = height - INPUT_ROWS;

        type_text(&app, window_id, input_id, "hello world");

        // Before submission the draft renders inside the input frame and the
        // cursor trails the typed text (one cell past it, offset by the border).
        let frame = app.update(|ctx| presenter.present(ctx, &root, area));
        let lines = frame.buffer.to_lines();
        assert!(
            lines[input_text_row as usize].contains("hello world"),
            "the draft should render in the input frame's text row:\n{}",
            lines.join("\n")
        );
        // 11 chars typed, +1 for the left border cell.
        assert_eq!(frame.cursor, Some((12, input_text_row)));

        submit(&app, window_id, input_id);

        let frame = app.update(|ctx| presenter.present(ctx, &root, area));
        let lines = frame.buffer.to_lines();

        // The submitted text moved into the transcript, which sits above the
        // input frame.
        let transcript_row =
            row_with(&lines, "hello world").expect("the transcript should show the submitted text");
        assert!(
            transcript_row < first_input_row as usize,
            "the submitted text should render above the input frame (row {transcript_row}):\n{}",
            lines.join("\n")
        );

        // The input is cleared back to its placeholder empty-state, and the
        // submitted text is no longer in the input frame.
        let input_region = lines[first_input_row as usize..].join("\n");
        assert!(
            input_region.contains("Warp anything"),
            "the cleared input should show its placeholder:\n{input_region}"
        );
        assert!(
            !input_region.contains("hello world"),
            "the submitted text should no longer be in the input frame:\n{input_region}"
        );

        // Focus stays on the input and the cursor resets to the frame's start.
        assert_eq!(app.focused_view_id(window_id), Some(input_id));
        assert_eq!(frame.cursor, Some((1, input_text_row)));
    });
}

#[test]
fn tui_initialize_app_registers_agent_singletons_without_terminal_session() {
    App::test((), |mut app| async move {
        initialize_tui_app(&mut app);

        assert_eq!(app.models_of_type::<CoreTuiModel>().len(), 1);
        assert_eq!(app.models_of_type::<BlocklistAIHistoryModel>().len(), 1);
        assert_eq!(app.models_of_type::<LLMPreferences>().len(), 1);
        assert_eq!(app.models_of_type::<AIExecutionProfilesModel>().len(), 1);
        assert_eq!(app.models_of_type::<BlocklistAIPermissions>().len(), 1);
        assert_eq!(app.models_of_type::<ServerApiProvider>().len(), 1);
        assert_eq!(app.models_of_type::<NetworkStatus>().len(), 1);
        assert_eq!(app.models_of_type::<TemplatableMCPServerManager>().len(), 1);
        assert!(app.models_of_type::<ActiveSession>().is_empty());
        assert!(app.models_of_type::<DefaultTerminal>().is_empty());
    });
}

#[test]
fn transcript_anchors_newest_entry_to_the_bottom_and_clips_the_top() {
    App::test((), |mut app| async move {
        let (window_id, root) =
            app.update(|ctx| ctx.add_tui_window(window_options(), RootTuiView::new));
        let input_id = app.read(|ctx| root.read(ctx, |view, _| view.input.id()));

        for entry in ["one", "two", "three", "four", "five"] {
            type_text(&app, window_id, input_id, entry);
            submit(&app, window_id, input_id);
        }

        // 7 rows tall leaves only 4 transcript rows above the 3-row input
        // frame, too few to show all five entries (each entry takes a text row
        // plus a spacer), so the oldest are clipped off the top.
        let mut presenter = TuiPresenter::new();
        let area = TuiRect::new(0, 0, 40, 7);
        let frame = app.update(|ctx| presenter.present(ctx, &root, area));
        let lines = frame.buffer.to_lines();
        let rendered = lines.join("\n");

        // The two newest entries are visible, oldest-above-newest (the newest
        // sits closest to the input).
        let four_row = row_with(&lines, "four").expect("the second-newest entry should be visible");
        let five_row = row_with(&lines, "five").expect("the newest entry should be visible");
        assert!(
            four_row < five_row,
            "newer entries should sit below older ones:\n{rendered}"
        );

        // The entries that overflow the top are clipped.
        for clipped in ["one", "two", "three"] {
            assert!(
                row_with(&lines, clipped).is_none(),
                "{clipped:?} should be clipped off the top:\n{rendered}"
            );
        }
    });
}

#[test]
fn core_tui_model_sends_initial_prompt_and_follow_up() {
    App::test((), |mut app| async move {
        initialize_tui_app(&mut app);
        let owner = AgentSessionOwnerId::new(EntityId::new());
        CoreTuiModel::handle(&app).update(&mut app, |model, ctx| {
            model.register_session(owner, ctx);
        });

        let (conversation_id, first_stream_id) = CoreTuiModel::handle(&app)
            .update(&mut app, |model, ctx| {
                model.send_prompt("first".to_string(), ctx)
            })
            .expect("first prompt should send");
        let first_params = in_flight_request_params(&app);
        assert_eq!(first_params.input.len(), 1);
        assert_eq!(first_params.conversation_token, None);
        assert_eq!(first_params.tasks.len(), 0);
        assert_eq!(first_params.supported_tools_override, Some(vec![]));

        wait_for_tui_request_to_finish(&mut app).await;

        BlocklistAIHistoryModel::handle(&app).read(&app, |history_model, _| {
            let conversation = history_model
                .conversation(&conversation_id)
                .expect("conversation should exist after first prompt");
            assert_eq!(conversation.root_task_exchanges().count(), 1);
            assert!(conversation.server_conversation_token().is_some());
            assert!(conversation
                .root_task_exchanges()
                .last()
                .expect("first exchange should exist")
                .output_status
                .is_finished_and_successful());
            assert!(conversation
                .new_exchange_ids_for_response(&first_stream_id)
                .next()
                .is_none());
        });

        let (follow_up_conversation_id, _second_stream_id) = CoreTuiModel::handle(&app)
            .update(&mut app, |model, ctx| {
                model.send_prompt("second".to_string(), ctx)
            })
            .expect("second prompt should send");
        assert_eq!(follow_up_conversation_id, conversation_id);
        let second_params = in_flight_request_params(&app);
        assert_eq!(second_params.input.len(), 1);
        assert!(
            second_params.conversation_token.is_some(),
            "follow-up should carry server conversation token",
        );
        assert!(
            !second_params.tasks.is_empty(),
            "follow-up should carry prior task context",
        );
        assert_eq!(second_params.supported_tools_override, Some(vec![]));

        wait_for_tui_request_to_finish(&mut app).await;

        BlocklistAIHistoryModel::handle(&app).read(&app, |history_model, _| {
            let conversations = history_model
                .all_live_conversations_for_terminal_view(owner.entity_id())
                .collect::<Vec<_>>();
            assert_eq!(conversations.len(), 1);
            let conversation = conversations[0];
            assert_eq!(conversation.id(), conversation_id);
            assert_eq!(conversation.root_task_exchanges().count(), 2);
            assert!(conversation
                .root_task_exchanges()
                .last()
                .expect("second exchange should exist")
                .output_status
                .is_finished_and_successful());
        });
    });
}
