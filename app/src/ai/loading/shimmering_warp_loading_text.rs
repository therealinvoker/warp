//! Shimmering loading text - renders the Bang lightning logo alongside shimmering
//! text for loading states.

use warp_core::ui::appearance::Appearance;
use warp_core::ui::Icon;
use warpui::elements::shimmering_text::{
    ShimmerConfig, ShimmeringTextElement, ShimmeringTextStateHandle,
};
use warpui::elements::{ConstrainedBox, CrossAxisAlignment, Element, Flex, ParentElement};
use warpui::{AppContext, SingletonEntity};

/// Horizontal gap (px) between the lightning logo and the loading text.
pub const LIGHTNING_LOADING_ICON_GAP: f32 = 4.;

/// Size (px) of the lightning logo rendered inline with loading text of the
/// given `font_size`. Kept as a helper so callers that need to visually align
/// other content (e.g. an indented tip) can reserve the matching width.
pub fn lightning_loading_icon_size(font_size: f32) -> f32 {
    font_size
}

/// Creates a shimmering text element preceded by the lightning logo.
pub fn shimmering_warp_loading_text(
    text: impl Into<String>,
    font_size: f32,
    shimmer_handle: ShimmeringTextStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    // Use same colors as common.rs for consistency
    let base_color = theme.disabled_text_color(theme.surface_1()).into_solid();
    let shimmer_color = theme.main_text_color(theme.surface_1()).into_solid();

    // Hardcoded shimmer config for consistent animation
    let config = ShimmerConfig::default();

    let icon_size = lightning_loading_icon_size(font_size);
    let lightning = ConstrainedBox::new(Icon::Lightning.to_warpui_icon(base_color.into()).finish())
        .with_width(icon_size)
        .with_height(icon_size)
        .finish();

    let shimmer = ShimmeringTextElement::new(
        text.into(),
        appearance.ui_font_family(),
        font_size,
        base_color,
        shimmer_color,
        config,
        shimmer_handle,
    )
    .finish();

    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(LIGHTNING_LOADING_ICON_GAP)
        .with_child(lightning)
        .with_child(shimmer)
        .finish()
}
