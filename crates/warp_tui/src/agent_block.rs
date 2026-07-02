//! An agent block in the TUI transcript: one exchange rendered as the user's
//! submitted input followed by the agent's response.
use std::collections::HashMap;
use std::rc::Rc;

use warp::tui_export::{
    AIAgentAction, AIAgentActionId, AIAgentActionType, AIAgentExchangeId, AIAgentOutputMessageType,
    AIAgentTextSection, AIBlockModel, AIConversationId, Appearance, BlocklistAIActionModel,
};
use warp_core::ui::color::blend::Blend;
// `ThemeFill` is the theme-layer color (it supports blend/opacity); `Fill` below
// is the element-layer color it converts into on its way to a terminal cell.
use warp_core::ui::theme::Fill as ThemeFill;
use warpui::SingletonEntity;
use warpui_core::elements::tui::{
    Modifier, TuiChildView, TuiConstraint, TuiContainer, TuiElement, TuiFlex, TuiLayoutContext,
    TuiParentElement, TuiSize, TuiStyle, TuiText,
};
use warpui_core::elements::Fill;
use warpui_core::{
    AppContext, Entity, EntityId, EntityIdMap, ModelHandle, TuiView, ViewContext, ViewHandle,
};

use super::tui_file_edits_view::TuiFileEditsView;

const INPUT_PREFIX: &str = "≫ ";

/// Renderable pieces of an agent block; this will grow as we render richer sections.
#[derive(Clone, Debug, Eq, PartialEq)]
enum TuiAIBlockSection {
    Input(String),
    PlainText(String),
    /// A lightweight status row standing in for an agent tool call.
    ToolCall(Box<AIAgentAction>),
}

/// A registered per-action child view for a stateful tool call.
///
/// Stateless tool calls render as pure elements in
/// [`TuiAIBlockSection::render_element`]; a tool type gets a variant here only
/// when it needs owned state or interactivity.
enum TuiToolCallView {
    FileEdits(ViewHandle<TuiFileEditsView>),
}

impl TuiToolCallView {
    /// The registered view's entity id, for [`TuiView::child_view_ids`].
    fn view_id(&self) -> EntityId {
        match self {
            Self::FileEdits(view) => view.id(),
        }
    }

    /// Renders the registered child view into the block's element tree.
    fn render_child(&self) -> TuiChildView {
        match self {
            Self::FileEdits(view) => TuiChildView::new(view),
        }
    }
}

/// A thin TUI rich-content view adapter backed by one agent exchange.
///
/// The rendering logic is mostly section extraction, but the shared block list
/// stores rich content by view id, so this remains a registered view.
pub(super) struct TuiAIBlock {
    conversation_id: AIConversationId,
    exchange_id: AIAgentExchangeId,
    model: Rc<dyn AIBlockModel<View = Self>>,
    /// Stateful per-action child views, keyed by tool-call action id.
    /// Populated by [`Self::sync_action_views`]; stateless tool calls never
    /// get entries here.
    action_views: HashMap<AIAgentActionId, TuiToolCallView>,
}

/// Extracts model state into renderable agent block sections.
impl TuiAIBlock {
    /// Creates an exchange-backed agent block. Like the GUI `AIBlock`, the
    /// block wires itself to its model at construction: it syncs per-action
    /// child views for tool calls already present, then re-syncs whenever the
    /// exchange's output updates (via `on_updated_output`).
    pub(super) fn new(
        conversation_id: AIConversationId,
        exchange_id: AIAgentExchangeId,
        model: Rc<dyn AIBlockModel<View = Self>>,
        action_model: ModelHandle<BlocklistAIActionModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let mut block = Self {
            conversation_id,
            exchange_id,
            model,
            action_views: HashMap::new(),
        };
        block.sync_action_views(&action_model, ctx);
        block.model.on_updated_output(
            Box::new(move |me, ctx| {
                me.sync_action_views(&action_model, ctx);
            }),
            ctx,
        );
        block
    }

    /// Creates child views for stateful tool calls that don't have one yet.
    /// Rendering can't create views since it only sees `&AppContext`.
    fn sync_action_views(
        &mut self,
        action_model: &ModelHandle<BlocklistAIActionModel>,
        ctx: &mut ViewContext<Self>,
    ) {
        let status = self.model.status(ctx);
        let file_edit_action_ids: Vec<AIAgentActionId> = status
            .output_to_render()
            .map(|output| {
                output
                    .get()
                    .messages
                    .iter()
                    .filter_map(|message| {
                        let AIAgentOutputMessageType::Action(action) = &message.message else {
                            return None;
                        };
                        matches!(action.action, AIAgentActionType::RequestFileEdits { .. })
                            .then(|| action.id.clone())
                    })
                    .collect()
            })
            .unwrap_or_default();

        for action_id in file_edit_action_ids {
            if self.action_views.contains_key(&action_id) {
                continue;
            }
            let view =
                ctx.add_tui_view(|ctx| TuiFileEditsView::new(action_id.clone(), action_model, ctx));
            self.action_views
                .insert(action_id, TuiToolCallView::FileEdits(view));
            ctx.notify();
        }
    }

    /// Replaces the backing model when the same exchange is reassigned.
    pub(super) fn replace_model(
        &mut self,
        conversation_id: AIConversationId,
        model: Rc<dyn AIBlockModel<View = Self>>,
    ) {
        self.conversation_id = conversation_id;
        self.model = model;
    }

    /// Returns the conversation that currently owns this agent block.
    pub(super) fn conversation_id(&self) -> AIConversationId {
        self.conversation_id
    }

    /// Returns the exchange rendered by this agent block.
    pub(super) fn exchange_id(&self) -> AIAgentExchangeId {
        self.exchange_id
    }

    /// Returns this block's wrapped height at the given width.
    pub(super) fn desired_height(&self, width: u16, app: &AppContext) -> usize {
        let mut rendered_views = EntityIdMap::default();
        let mut ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        let mut element = self.render_element(app);
        usize::from(
            element
                .layout(
                    TuiConstraint::loose(TuiSize::new(width, u16::MAX)),
                    &mut ctx,
                    app,
                )
                .height,
        )
    }

    /// Extracts this exchange's visible input/output into logical render sections.
    fn sections(&self, app: &AppContext) -> Vec<TuiAIBlockSection> {
        let mut sections = Vec::new();
        let input = self
            .model
            .inputs_to_render(app)
            .iter()
            .filter_map(|input| input.display_query())
            .collect::<Vec<_>>()
            .join("\n");
        if !input.is_empty() {
            sections.push(TuiAIBlockSection::Input(input));
        }

        // Walk output messages in order so tool-call rows interleave with text.
        if let Some(output) = self.model.status(app).output_to_render() {
            let output = output.get();
            for message in &output.messages {
                match &message.message {
                    AIAgentOutputMessageType::Text(text) => {
                        sections.extend(text.sections.iter().filter_map(|section| {
                            match section {
                                AIAgentTextSection::PlainText { text } => (!text.text().is_empty())
                                    .then(|| TuiAIBlockSection::PlainText(text.text().to_owned())),
                                // Add item variants here as the TUI learns to render richer sections.
                                AIAgentTextSection::Code { .. }
                                | AIAgentTextSection::Table { .. }
                                | AIAgentTextSection::Image { .. }
                                | AIAgentTextSection::MermaidDiagram { .. } => None,
                            }
                        }));
                    }
                    AIAgentOutputMessageType::Action(action) => {
                        sections.push(TuiAIBlockSection::ToolCall(Box::new(action.clone())));
                    }
                    AIAgentOutputMessageType::Reasoning { .. }
                    | AIAgentOutputMessageType::Summarization { .. }
                    | AIAgentOutputMessageType::Subagent(_)
                    | AIAgentOutputMessageType::TodoOperation(_)
                    | AIAgentOutputMessageType::WebSearch(_)
                    | AIAgentOutputMessageType::WebFetch(_)
                    | AIAgentOutputMessageType::CommentsAddressed { .. }
                    | AIAgentOutputMessageType::DebugOutput { .. }
                    | AIAgentOutputMessageType::ArtifactCreated(_)
                    | AIAgentOutputMessageType::SkillInvoked(_)
                    | AIAgentOutputMessageType::MessagesReceivedFromAgents { .. }
                    | AIAgentOutputMessageType::EventsFromAgents { .. } => {}
                }
            }
        }

        sections
    }

    /// Builds this block's generic TUI element tree.
    fn render_element(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let sections = self.sections(app);

        let mut column = TuiFlex::column();
        for (index, section) in sections.iter().enumerate() {
            // Output is many sections (one per text section), so top padding is
            // applied only to the section right after the input, giving a single
            // gap at the input→output boundary rather than before every line.
            let follows_input = index
                .checked_sub(1)
                .is_some_and(|prev| matches!(sections[prev], TuiAIBlockSection::Input(_)));
            let top_padding = u16::from(follows_input);
            // Stateful tool calls render their registered child view; every
            // other section stays a pure render fn.
            let element = match section {
                TuiAIBlockSection::ToolCall(action) => match self.action_views.get(&action.id) {
                    Some(view) => TuiContainer::new(view.render_child())
                        .with_padding_top(top_padding)
                        .finish(),
                    None => section.render_element(top_padding, app),
                },
                TuiAIBlockSection::Input(_) | TuiAIBlockSection::PlainText(_) => {
                    section.render_element(top_padding, app)
                }
            };
            column = column.with_child(element);
        }

        // No background of its own: the block shows the terminal's background,
        // matching the Figma where only the input line is highlighted.
        TuiContainer::new(column)
            .with_padding_bottom(u16::from(!sections.is_empty()))
            .finish()
    }
}

/// Converts one logical section into a renderable TUI element.
impl TuiAIBlockSection {
    fn render_element(&self, top_padding: u16, app: &AppContext) -> Box<dyn TuiElement> {
        let theme = Appearance::as_ref(app).theme();
        match self {
            Self::Input(text) => {
                let text_color = Fill::from(theme.foreground()).into();
                let accent = ThemeFill::from(theme.terminal_colors().normal.cyan);
                let background = Fill::from(
                    theme
                        .background()
                        .blend(&accent.with_opacity(10))
                        .blend(&accent.with_opacity(10)),
                )
                .into();
                // Only the first line carries the `≫` prompt marker; continuation
                // lines are indented to the marker's width so they align beneath it.
                let mut column = TuiFlex::column();
                for (index, line) in text.split('\n').enumerate() {
                    let line_text = if index == 0 {
                        format!("{INPUT_PREFIX}{line}")
                    } else {
                        format!("{}{line}", " ".repeat(INPUT_PREFIX.chars().count()))
                    };
                    column = column.child(
                        TuiText::new(line_text)
                            .with_style(
                                TuiStyle::default()
                                    .fg(text_color)
                                    .bg(background)
                                    .add_modifier(Modifier::BOLD),
                            )
                            .finish(),
                    );
                }
                TuiContainer::new(column)
                    .with_background(background)
                    .with_padding_top(top_padding)
                    .finish()
            }
            Self::PlainText(text) => {
                let text_color =
                    Fill::from(ThemeFill::from(theme.terminal_colors().normal.white)).into();
                TuiContainer::new(
                    TuiText::new(text.clone()).with_style(TuiStyle::default().fg(text_color)),
                )
                .with_padding_top(top_padding)
                .finish()
            }
            Self::ToolCall(_action) => {
                // TODO: add richer rendering for each tool call type. This is just a rendering stub to build off of.
                let text_color =
                    Fill::from(ThemeFill::from(theme.terminal_colors().bright.black)).into();
                TuiContainer::new(
                    TuiText::new("executed a tool call").with_style(
                        TuiStyle::default()
                            .fg(text_color)
                            .add_modifier(Modifier::DIM),
                    ),
                )
                .with_padding_top(top_padding)
                .finish()
            }
        }
    }
}

/// Registers the view with the TUI runtime.
impl Entity for TuiAIBlock {
    type Event = ();
}

/// Renders the model-backed block as a TUI element.
impl TuiView for TuiAIBlock {
    fn ui_name() -> &'static str {
        "TuiAIBlock"
    }

    fn child_view_ids(&self, _app: &AppContext) -> Vec<EntityId> {
        self.action_views
            .values()
            .map(|view| view.view_id())
            .collect()
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        self.render_element(app)
    }
}

#[cfg(test)]
#[path = "agent_block_tests.rs"]
mod tests;
