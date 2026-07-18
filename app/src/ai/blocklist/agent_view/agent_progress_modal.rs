//! Body of the read-only live per-agent progress modal.
//!
//! Hosted by the workspace inside a generic [`crate::modal::Modal`], this view
//! shows a single orchestration child ("worker") conversation's live progress:
//! its current status, an "Open in pane" affordance, and a read-only,
//! live-updating transcript of the worker's turn. It never mutates the child —
//! it reads the conversation from the global [`BlocklistAIHistoryModel`] and
//! re-renders whenever the history model changes, so the transcript streams in
//! as the worker makes progress.

use warpui::elements::{
    ClippedScrollStateHandle, ClippedScrollable, Container, CrossAxisAlignment, Empty, Expanded,
    Fill, Flex, Hoverable, MainAxisSize, MouseStateHandle, ParentElement, ScrollbarWidth, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::agent_view::orchestration_conversation_links::conversation_navigation_action;
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::appearance::Appearance;
use crate::modal::ModalAction;

const SCROLLBAR_WIDTH: f32 = 6.;

/// Actions dispatched by the progress modal body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentProgressModalBodyAction {
    /// Open the worker's conversation in its own pane and close the modal.
    OpenInPane,
}

pub struct AgentProgressModalBody {
    /// The worker conversation currently being viewed. `None` before the modal
    /// has been pointed at a specific worker.
    conversation_id: Option<AIConversationId>,
    scroll_state: ClippedScrollStateHandle,
    open_in_pane_mouse_state: MouseStateHandle,
}

impl AgentProgressModalBody {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let history = BlocklistAIHistoryModel::handle(ctx);
        // Re-render on any history change so the worker's transcript and status
        // stream in live while the modal is open.
        ctx.subscribe_to_model(&history, |_me, _, _event: &BlocklistAIHistoryEvent, ctx| {
            ctx.notify();
        });
        Self {
            conversation_id: None,
            scroll_state: Default::default(),
            open_in_pane_mouse_state: Default::default(),
        }
    }

    /// Points the modal at a specific worker conversation (or clears it).
    pub fn set_conversation(
        &mut self,
        conversation_id: Option<AIConversationId>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.conversation_id = conversation_id;
        ctx.notify();
    }

    fn render_header(&self, status: &str, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let status_text = Text::new(
            format!("Status: {status}"),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(theme.disabled_text_color(theme.background()).into())
        .finish();

        let open_in_pane = Hoverable::new(self.open_in_pane_mouse_state.clone(), move |_| {
            Text::new(
                "Open in pane ›".to_string(),
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(theme.main_text_color(theme.background()).into())
            .with_style(Properties::default().weight(Weight::Bold))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(AgentProgressModalBodyAction::OpenInPane);
        })
        .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(status_text)
            .with_child(Expanded::new(1., Empty::new().finish()).finish())
            .with_child(open_in_pane)
            .finish()
    }
}

impl Entity for AgentProgressModalBody {
    type Event = ();
}

impl TypedActionView for AgentProgressModalBody {
    type Action = AgentProgressModalBodyAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AgentProgressModalBodyAction::OpenInPane => {
                if let Some(conversation_id) = self.conversation_id {
                    if let Some(navigation) = conversation_navigation_action(conversation_id, ctx) {
                        ctx.dispatch_typed_action(&navigation);
                    }
                }
                // Close the modal (handled by the enclosing `Modal`).
                ctx.dispatch_typed_action(&ModalAction::Close);
            }
        }
    }
}

impl View for AgentProgressModalBody {
    fn ui_name() -> &'static str {
        "AgentProgressModalBody"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let Some(conversation_id) = self.conversation_id else {
            return Empty::new().finish();
        };
        let history = BlocklistAIHistoryModel::as_ref(app);
        let Some(conversation) = history.conversation(&conversation_id) else {
            return Empty::new().finish();
        };

        let status = conversation.status().to_string();
        // Read-only transcript of the worker's turn. `None` for the action
        // model keeps this a lightweight text view (agent narration + user
        // prompt) without pulling in tool-call rendering machinery.
        let transcript = conversation.export_to_markdown(None);

        let header = self.render_header(&status, app);

        let transcript_text = Text::new(
            transcript,
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(theme.main_text_color(theme.background()).into())
        .finish();

        let scrollable = ClippedScrollable::vertical(
            self.scroll_state.clone(),
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(transcript_text)
                .finish(),
            ScrollbarWidth::Custom(SCROLLBAR_WIDTH),
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            Fill::None,
        )
        .finish();

        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(10.)
                .with_child(header)
                .with_child(Expanded::new(1., scrollable).finish())
                .finish(),
        )
        .with_uniform_padding(16.)
        .finish()
    }
}
