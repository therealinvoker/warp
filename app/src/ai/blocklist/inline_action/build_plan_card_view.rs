//! Inline card shown after the agent finishes writing a plan document in
//! `/plan` mode. Offers two ways to act on the plan:
//! - **Build locally**: continue this conversation with a normal (non-plan)
//!   follow-up query so the agent may now edit files and run commands.
//! - **Build in cloud**: hand the plan off to a fresh cloud agent.
//!
//! Modeled on [`RunAgentsCardView`](super::run_agents_card_view): the card is a
//! `View` keyed by `AIAgentActionId` and embedded by `AIBlock` via `ChildView`.
//! Both buttons emit an event the parent `AIBlock` re-emits as an `AIBlockEvent`
//! so `TerminalView` can perform the actual build.
use warpui::elements::{
    Border, ChildView, Container, CornerRadius, CrossAxisAlignment, Flex, MainAxisSize,
    ParentElement, Radius, Text,
};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ai::blocklist::block::view_impl::WithContentItemSpacing;
use crate::appearance::Appearance;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ButtonSize, NakedTheme};

const CARD_TITLE: &str = "Plan ready. Build it?";

#[derive(Clone, Debug)]
pub enum BuildPlanCardViewAction {
    BuildLocally,
    BuildInCloud,
}

#[derive(Clone, Debug)]
pub enum BuildPlanCardViewEvent {
    BuildLocallyRequested,
    BuildInCloudRequested,
}

pub struct BuildPlanCardView {
    build_locally_button: ViewHandle<ActionButton>,
    build_in_cloud_button: ViewHandle<ActionButton>,
}

impl BuildPlanCardView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let build_locally_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Build locally", NakedTheme)
                .with_icon(Icon::Terminal)
                .with_size(ButtonSize::Small)
                .with_tooltip("Continue in this session and start implementing the plan")
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(BuildPlanCardViewAction::BuildLocally);
                })
        });
        let build_in_cloud_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Build in cloud", NakedTheme)
                .with_icon(Icon::Cloud)
                .with_size(ButtonSize::Small)
                .with_tooltip("Hand the plan off to a cloud agent")
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(BuildPlanCardViewAction::BuildInCloud);
                })
        });
        Self {
            build_locally_button,
            build_in_cloud_button,
        }
    }
}

impl Entity for BuildPlanCardView {
    type Event = BuildPlanCardViewEvent;
}

impl View for BuildPlanCardView {
    fn ui_name() -> &'static str {
        "BuildPlanCardView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let title = Text::new(
            CARD_TITLE.to_string(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(blended_colors::text_main(theme, theme.background()))
        .finish();

        let buttons = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(ChildView::new(&self.build_locally_button).finish())
            .with_child(ChildView::new(&self.build_in_cloud_button).finish())
            .finish();

        let column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(Container::new(title).with_margin_bottom(8.).finish())
            .with_child(buttons)
            .finish();

        Container::new(column)
            .with_horizontal_padding(16.)
            .with_vertical_padding(12.)
            .with_background_color(theme.background().into_solid())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_border(Border::all(1.).with_border_fill(theme.accent()))
            .finish()
            .with_content_item_spacing()
            .finish()
    }
}

impl TypedActionView for BuildPlanCardView {
    type Action = BuildPlanCardViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            BuildPlanCardViewAction::BuildLocally => {
                ctx.emit(BuildPlanCardViewEvent::BuildLocallyRequested);
            }
            BuildPlanCardViewAction::BuildInCloud => {
                ctx.emit(BuildPlanCardViewEvent::BuildInCloudRequested);
            }
        }
    }
}
