use warp_core::ui::theme::{Fill, WarpTheme};
use warp_core::ui::Icon;
use warpui::Element;

pub mod conversation_usage_view;
pub mod rollup;

pub fn icon_for_context_window_usage(context_window_usage: f32) -> Icon {
    // The ring's solid (white) arc represents the context *used*, not the
    // amount remaining: an empty conversation shows just the dim track (0%
    // used) and the bright arc sweeps clockwise to a full ring as the context
    // window fills (100% used). So match the *used* fraction directly to the
    // nearest 10% icon, where `ContextRemainingN` fills N% of the ring. The
    // filled (white) sweep therefore equals the used fraction (`1 - remaining`).
    if context_window_usage >= 0.95 {
        Icon::ContextRemaining100
    } else if context_window_usage >= 0.85 {
        Icon::ContextRemaining90
    } else if context_window_usage >= 0.75 {
        Icon::ContextRemaining80
    } else if context_window_usage >= 0.65 {
        Icon::ContextRemaining70
    } else if context_window_usage >= 0.55 {
        Icon::ContextRemaining60
    } else if context_window_usage >= 0.45 {
        Icon::ContextRemaining50
    } else if context_window_usage >= 0.35 {
        Icon::ContextRemaining40
    } else if context_window_usage >= 0.25 {
        Icon::ContextRemaining30
    } else if context_window_usage >= 0.15 {
        Icon::ContextRemaining20
    } else if context_window_usage >= 0.05 {
        Icon::ContextRemaining10
    } else {
        Icon::ContextRemaining0
    }
}

pub fn render_context_window_usage_icon(
    context_window_usage: f32,
    theme: &WarpTheme,
    color_override: Option<Fill>,
) -> Box<dyn Element> {
    let icon = icon_for_context_window_usage(context_window_usage);

    let fill = if context_window_usage >= 0.8 {
        Fill::Solid(theme.ansi_fg_red())
    } else {
        color_override.unwrap_or_else(|| theme.main_text_color(theme.background()))
    };

    icon.to_warpui_icon(fill).finish()
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
