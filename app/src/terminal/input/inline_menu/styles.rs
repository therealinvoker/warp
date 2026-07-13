//! Shared styling utilities for inline menu implementations.
//!
//! This module provides common styling functions used across all inline menu
//! implementations (models, slash commands, conversations) to ensure consistent
//! visual design matching the Figma specifications.
use warp_core::ui::appearance::Appearance;
use warp_core::ui::color::blend::Blend;
use warp_core::ui::theme::{Fill, WarpTheme};
use warpui::color::ColorU;
use warpui::{AppContext, SingletonEntity};

use crate::ai::blocklist::agent_view::agent_view_bg_fill;
use crate::search::result_renderer::ItemHighlightState;

/// Font size used for inline menu items (slash commands, models, profiles,
/// prompts, skills, history, etc.).
///
/// The fork shrank the base monospace font by 2pts; render the menu rows at the
/// full monospace size (2pts larger than the historical `monospace - 2`) so the
/// command list and other menus stay readable. The row height
/// (`result_item_height_fn` in `view.rs`) is bumped in step so the taller glyphs
/// don't clip.
pub fn font_size(appearance: &Appearance) -> f32 {
    appearance.monospace_font_size()
}

/// Font size used for the inline menu navigation/hint bar ("↑ ↓ to navigate",
/// "esc to dismiss"). Kept 2pts above the row font so the hint line stays a touch
/// larger than the list, matching the pre-fork proportions. Scoped to inline
/// menus so the terminal/agent status message bars keep their own size.
pub fn message_bar_font_size(appearance: &Appearance) -> f32 {
    font_size(appearance) + 2.
}

pub const ICON_MARGIN: f32 = 8.0;
pub const ITEM_HORIZONTAL_PADDING: f32 = 8.0;
pub const ITEM_CORNER_RADIUS: f32 = 4.0;
pub const CONTENT_VERTICAL_PADDING: f32 = 8.;
pub const CONTENT_BORDER_WIDTH: f32 = 1.;

/// Height of the header row content, ensuring all headers render at the same
/// height regardless of whether tabs or trailing elements are present.
pub const HEADER_ROW_HEIGHT: f32 = 24.;
pub const HEADER_BORDER: f32 = 1.;

pub fn menu_background_color(app: &AppContext) -> ColorU {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    theme.background().blend(&agent_view_bg_fill(app)).into()
}

pub fn item_background(
    highlight_state: ItemHighlightState,
    appearance: &Appearance,
) -> Option<Fill> {
    let theme = appearance.theme();
    match highlight_state {
        ItemHighlightState::Selected { .. } => Some(theme.surface_overlay_2()),
        ItemHighlightState::Hovered => Some(theme.surface_overlay_1()),
        ItemHighlightState::Default => None,
    }
}

pub fn primary_text_color(theme: &WarpTheme, background: Fill) -> Fill {
    theme.main_text_color(background)
}

pub fn secondary_text_color(theme: &WarpTheme, background: Fill) -> Fill {
    theme.sub_text_color(background)
}

pub fn disabled_text_color(theme: &WarpTheme, background: Fill) -> Fill {
    theme.disabled_text_color(background)
}

pub fn icon_color(appearance: &Appearance) -> Fill {
    let theme = appearance.theme();
    theme.sub_text_color(theme.background()).with_opacity(80)
}
