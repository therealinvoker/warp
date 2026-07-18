//! Composer-anchored "N Working" indicator.
//!
//! Rendered directly above the agent composer, this view surfaces the
//! orchestration workers spawned under the active conversation's orchestrator.
//! Collapsed, it is a small pill showing how many workers are still running;
//! expanded (on click), it lists every worker by its task label, each row
//! opening the read-only progress modal and offering a per-agent Stop, plus a
//! "Stop all". It replaces the top orchestration pill bar behind
//! [`FeatureFlag::AgentProgressUI`].

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty, Expanded, Flex, Hoverable,
    MainAxisSize, MouseStateHandle, ParentElement, Radius, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::ai::agent::conversation::{AIConversationId, StatusColorStyle};
use crate::ai::blocklist::agent_view::AgentViewController;
use crate::ai::blocklist::orchestration_topology::{
    orchestration_workers, worker_status_is_active, OrchestrationWorker, OrchestrationWorkers,
};
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::features::FeatureFlag;
use crate::terminal::view::TerminalAction;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::workspace::WorkspaceAction;

const ROW_LABEL_MAX_WIDTH: f32 = 260.;
const PILL_HEIGHT: f32 = 24.;
const ICON_SIZE: f32 = 14.;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkingAgentsIndicatorAction {
    /// Toggle the expanded worker list.
    ToggleExpanded,
    /// Open the read-only progress modal for a worker.
    OpenAgent(AIConversationId),
    /// Stop a single worker.
    StopAgent(AIConversationId),
    /// Stop every still-running worker.
    StopAll,
}

pub struct WorkingAgentsIndicator {
    agent_view_controller: ModelHandle<AgentViewController>,
    expanded: bool,
    pill_mouse_state: MouseStateHandle,
    stop_all_mouse_state: MouseStateHandle,
    /// Per-worker hover state for the (clickable) row body.
    row_mouse_states: RefCell<HashMap<AIConversationId, MouseStateHandle>>,
    /// Per-worker hover state for the row's Stop button.
    stop_mouse_states: RefCell<HashMap<AIConversationId, MouseStateHandle>>,
}

impl WorkingAgentsIndicator {
    pub fn new(
        agent_view_controller: ModelHandle<AgentViewController>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let history = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history, |_me, _, _event: &BlocklistAIHistoryEvent, ctx| {
            ctx.notify();
        });
        Self {
            agent_view_controller,
            expanded: false,
            pill_mouse_state: Default::default(),
            stop_all_mouse_state: Default::default(),
            row_mouse_states: Default::default(),
            stop_mouse_states: Default::default(),
        }
    }

    fn workers(&self, app: &AppContext) -> Option<OrchestrationWorkers> {
        let active_id = self
            .agent_view_controller
            .as_ref(app)
            .agent_view_state()
            .active_conversation_id()?;
        orchestration_workers(BlocklistAIHistoryModel::as_ref(app), active_id)
    }

    /// Keeps the per-row hover-state maps in sync with the live worker set so
    /// handles don't leak when workers come and go.
    fn prune_mouse_states(&self, workers: &OrchestrationWorkers) {
        let alive: HashSet<AIConversationId> =
            workers.workers.iter().map(|w| w.conversation_id).collect();
        let mut rows = self.row_mouse_states.borrow_mut();
        let mut stops = self.stop_mouse_states.borrow_mut();
        for id in &alive {
            rows.entry(*id).or_default();
            stops.entry(*id).or_default();
        }
        rows.retain(|id, _| alive.contains(id));
        stops.retain(|id, _| alive.contains(id));
    }

    fn render_pill(&self, workers: &OrchestrationWorkers, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let working_count = workers.working_count();
        let expanded = self.expanded;

        let label = if working_count > 0 {
            format!("{working_count} Working")
        } else {
            "Agents".to_string()
        };
        let (status_icon, status_color) = if working_count > 0 {
            (Icon::ClockLoader, theme.ansi_fg_magenta())
        } else {
            (Icon::Check, theme.ansi_fg_green())
        };
        let chevron = if expanded {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };

        Hoverable::new(self.pill_mouse_state.clone(), move |hover_state| {
            // Solid (opaque) chip fill so the pill reads as a distinct control
            // above the composer; a 5% foreground overlay was effectively
            // invisible against the composer background.
            let background = if hover_state.is_hovered() {
                blended_colors::neutral_3(theme)
            } else {
                blended_colors::neutral_2(theme)
            };
            let row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(6.)
                .with_child(
                    ConstrainedBox::new(
                        status_icon
                            .to_warpui_icon(Fill::Solid(status_color))
                            .finish(),
                    )
                    .with_width(ICON_SIZE)
                    .with_height(ICON_SIZE)
                    .finish(),
                )
                .with_child(
                    Text::new(
                        label.clone(),
                        appearance.ui_font_family(),
                        appearance.monospace_font_size(),
                    )
                    .with_color(blended_colors::text_main(theme, theme.background()))
                    .with_style(Properties::default().weight(Weight::Bold))
                    .soft_wrap(false)
                    .finish(),
                )
                .with_child(
                    ConstrainedBox::new(
                        chevron
                            .to_warpui_icon(Fill::Solid(blended_colors::text_sub(
                                theme,
                                theme.background(),
                            )))
                            .finish(),
                    )
                    .with_width(ICON_SIZE)
                    .with_height(ICON_SIZE)
                    .finish(),
                )
                .finish();
            Container::new(row)
                .with_background_color(background.into())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(PILL_HEIGHT / 2.)))
                .with_horizontal_padding(10.)
                .with_vertical_padding(4.)
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(WorkingAgentsIndicatorAction::ToggleExpanded);
        })
        .finish()
    }

    fn render_worker_row(
        &self,
        worker: &OrchestrationWorker,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let (status_icon, status_color) = worker
            .status
            .status_icon_and_color(theme, StatusColorStyle::Standard);
        let is_active = worker_status_is_active(&worker.status);
        let conversation_id = worker.conversation_id;
        let label = worker.label.clone();

        let row_mouse_state = self
            .row_mouse_states
            .borrow_mut()
            .entry(conversation_id)
            .or_default()
            .clone();

        let body = Hoverable::new(row_mouse_state, move |hover_state| {
            let background = if hover_state.is_hovered() {
                blended_colors::fg_overlay_2(theme)
            } else {
                Fill::Solid(ColorU::new(0, 0, 0, 0))
            };
            let row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Max)
                .with_spacing(8.)
                .with_child(
                    ConstrainedBox::new(
                        status_icon
                            .to_warpui_icon(Fill::Solid(status_color))
                            .finish(),
                    )
                    .with_width(ICON_SIZE)
                    .with_height(ICON_SIZE)
                    .finish(),
                )
                .with_child(
                    ConstrainedBox::new(
                        Text::new(
                            label.clone(),
                            appearance.ui_font_family(),
                            appearance.monospace_font_size(),
                        )
                        .with_color(blended_colors::text_main(theme, theme.background()))
                        .soft_wrap(false)
                        .with_clip(ClipConfig::ellipsis())
                        .finish(),
                    )
                    .with_max_width(ROW_LABEL_MAX_WIDTH)
                    .finish(),
                )
                .finish();
            Container::new(row)
                .with_background_color(background.into())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_horizontal_padding(8.)
                .with_vertical_padding(5.)
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorkingAgentsIndicatorAction::OpenAgent(conversation_id));
        })
        .finish();

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Expanded::new(1., body).finish());

        if is_active {
            let stop_mouse_state = self
                .stop_mouse_states
                .borrow_mut()
                .entry(conversation_id)
                .or_default()
                .clone();
            let stop_button = Hoverable::new(stop_mouse_state, move |hover_state| {
                let color = if hover_state.is_hovered() {
                    theme.ansi_fg_red()
                } else {
                    blended_colors::text_sub(theme, theme.background())
                };
                Container::new(
                    ConstrainedBox::new(
                        Icon::StopFilled.to_warpui_icon(Fill::Solid(color)).finish(),
                    )
                    .with_width(ICON_SIZE)
                    .with_height(ICON_SIZE)
                    .finish(),
                )
                .with_horizontal_padding(4.)
                .with_vertical_padding(4.)
                .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(WorkingAgentsIndicatorAction::StopAgent(conversation_id));
            })
            .finish();
            row.add_child(stop_button);
        }

        row.finish()
    }

    fn render_expanded_list(
        &self,
        workers: &OrchestrationWorkers,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(2.);

        if workers.working_count() > 0 {
            let stop_all = Hoverable::new(self.stop_all_mouse_state.clone(), move |hover_state| {
                let color = if hover_state.is_hovered() {
                    theme.ansi_fg_red()
                } else {
                    blended_colors::text_sub(theme, theme.background())
                };
                Container::new(
                    Text::new(
                        "Stop all".to_string(),
                        appearance.ui_font_family(),
                        (appearance.monospace_font_size() - 1.).max(10.),
                    )
                    .with_color(color)
                    .finish(),
                )
                .with_horizontal_padding(8.)
                .with_vertical_padding(4.)
                .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(WorkingAgentsIndicatorAction::StopAll);
            })
            .finish();
            column.add_child(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_child(Expanded::new(1., Empty::new().finish()).finish())
                    .with_child(stop_all)
                    .finish(),
            );
        }

        for worker in &workers.workers {
            column.add_child(self.render_worker_row(worker, app));
        }

        Container::new(column.finish())
            .with_background_color(blended_colors::neutral_2(theme).into())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .with_horizontal_padding(6.)
            .with_vertical_padding(6.)
            .with_margin_top(4.)
            .finish()
    }
}

impl Entity for WorkingAgentsIndicator {
    type Event = ();
}

impl TypedActionView for WorkingAgentsIndicator {
    type Action = WorkingAgentsIndicatorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            WorkingAgentsIndicatorAction::ToggleExpanded => {
                self.expanded = !self.expanded;
                ctx.notify();
            }
            WorkingAgentsIndicatorAction::OpenAgent(conversation_id) => {
                ctx.dispatch_typed_action(&WorkspaceAction::OpenAgentProgressModal {
                    conversation_id: *conversation_id,
                });
            }
            WorkingAgentsIndicatorAction::StopAgent(conversation_id) => {
                ctx.dispatch_typed_action(&TerminalAction::StopAgentConversation {
                    conversation_id: *conversation_id,
                });
            }
            WorkingAgentsIndicatorAction::StopAll => {
                if let Some(workers) = self.workers(ctx) {
                    for conversation_id in workers.active_worker_ids() {
                        ctx.dispatch_typed_action(&TerminalAction::StopAgentConversation {
                            conversation_id,
                        });
                    }
                }
            }
        }
    }
}

impl View for WorkingAgentsIndicator {
    fn ui_name() -> &'static str {
        "WorkingAgentsIndicator"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if !FeatureFlag::AgentProgressUI.is_enabled() {
            return Empty::new().finish();
        }
        let Some(workers) = self.workers(app) else {
            return Empty::new().finish();
        };
        self.prune_mouse_states(&workers);

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(self.render_pill(&workers, app));

        if self.expanded {
            column.add_child(self.render_expanded_list(&workers, app));
        }

        column.finish()
    }
}
