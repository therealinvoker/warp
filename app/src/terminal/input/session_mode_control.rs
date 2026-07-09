use pathfinder_color::ColorU;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::WarpTheme;
use warpui::elements::{
    Container, CornerRadius, CrossAxisAlignment, Element, Flex, Hoverable, MainAxisSize,
    MouseStateHandle, ParentElement, Radius, Text,
};
use warpui::platform::Cursor;
use warpui::prelude::Align;

use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::appearance::Appearance;
use crate::terminal::profile_model_selector::calculate_scaled_font_size;
use crate::terminal::view::TerminalAction;
use crate::themes::theme::Fill as ThemeFill;

/// The three session modes surfaced by the footer segmented control.
#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum SessionModeSegment {
    Agent,
    CloudAgent,
    Terminal,
}

impl SessionModeSegment {
    fn label(self) -> &'static str {
        match self {
            SessionModeSegment::Agent => "Agent",
            SessionModeSegment::CloudAgent => "Cloud Agent",
            SessionModeSegment::Terminal => "Terminal",
        }
    }

    /// The [`TerminalAction`] dispatched to switch the current pane to this mode.
    fn switch_action(self) -> TerminalAction {
        match self {
            SessionModeSegment::Agent => TerminalAction::SwitchToAgentView {
                origin: AgentViewEntryOrigin::Input {
                    was_prompt_autodetected: false,
                },
            },
            SessionModeSegment::CloudAgent => TerminalAction::EnterCloudAgentView,
            SessionModeSegment::Terminal => TerminalAction::ExitAgentView,
        }
    }
}

/// Mouse state handles for the segmented control. These must be created once during view
/// construction (per the `MouseStateHandle` guidance in AGENTS.md) and reused across renders.
#[derive(Default, Clone)]
pub(crate) struct SessionModeSegmentMouseStates {
    pub agent: MouseStateHandle,
    pub cloud_agent: MouseStateHandle,
    pub terminal: MouseStateHandle,
}

/// Renders a compact grouped segmented control with the segments ordered Agent, Cloud Agent
/// (only when `include_cloud_agent`), then Terminal. The `current` segment is highlighted;
/// clicking a different segment dispatches the mapped [`TerminalAction`], while clicking the
/// active segment is a no-op. This is a pure render helper; selection is derived from `current`.
pub(crate) fn render_session_mode_segmented_control(
    current: SessionModeSegment,
    include_cloud_agent: bool,
    states: &SessionModeSegmentMouseStates,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();

    let mut row = Flex::row()
        .with_main_axis_size(MainAxisSize::Min)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(render_segment(
            SessionModeSegment::Agent,
            current,
            states.agent.clone(),
            appearance,
            theme,
        ));

    if include_cloud_agent {
        row = row.with_child(render_segment(
            SessionModeSegment::CloudAgent,
            current,
            states.cloud_agent.clone(),
            appearance,
            theme,
        ));
    }

    row = row.with_child(render_segment(
        SessionModeSegment::Terminal,
        current,
        states.terminal.clone(),
        appearance,
        theme,
    ));

    Container::new(row.finish())
        .with_background(internal_colors::fg_overlay_2(theme))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .with_uniform_padding(2.)
        .finish()
}

fn render_segment(
    segment: SessionModeSegment,
    current: SessionModeSegment,
    mouse_state: MouseStateHandle,
    appearance: &Appearance,
    theme: &WarpTheme,
) -> Box<dyn Element> {
    let is_selected = segment == current;
    let label = segment.label().to_string();
    let action = segment.switch_action();
    let font_size = calculate_scaled_font_size(appearance);
    let main_text = theme.main_text_color(theme.background());
    let sub_text = theme.sub_text_color(theme.background());

    Hoverable::new(mouse_state, move |hover_state| {
        let background = if is_selected {
            internal_colors::fg_overlay_3(theme)
        } else if hover_state.is_hovered() {
            internal_colors::fg_overlay_1(theme)
        } else {
            ThemeFill::Solid(ColorU::transparent_black())
        };

        Container::new(
            Align::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(
                        Text::new_inline(label.clone(), appearance.ui_font_family(), font_size)
                            .with_color(if is_selected { main_text } else { sub_text }.into())
                            .finish(),
                    )
                    .finish(),
            )
            .finish(),
        )
        .with_background(background)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .with_horizontal_padding(6.)
        .with_vertical_padding(2.)
        .finish()
    })
    .on_click(move |ctx, _, _| {
        if !is_selected {
            ctx.dispatch_typed_action(action.clone());
        }
    })
    .with_cursor(Cursor::PointingHand)
    .finish()
}
