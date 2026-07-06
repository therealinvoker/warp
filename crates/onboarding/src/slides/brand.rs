//! Shared Bang brand primitives (colors, logo mark, accent button theme) used
//! across the onboarding slides so the rebrand stays consistent and DRY.

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use ui_components::button;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::{Fill, HorizontalGradient};
use warp_core::ui::Icon;
use warpui_core::elements::{Align, ConstrainedBox, Container};
use warpui_core::scene::{CornerRadius, Radius};
use warpui_core::{Element, Gradient};

/// The reference size (in px) the Bang mark was designed at. Corner radius and
/// the inner lightning glyph are scaled proportionally from these values so the
/// mark looks identical at 64px and scales cleanly to other sizes.
const REFERENCE_SIZE: f32 = 64.;
const REFERENCE_CORNER_RADIUS: f32 = 15.;
const REFERENCE_ICON_SIZE: f32 = 35.;

/// Bang brand pink (`#ff2d78`), the gradient's top-left / warm endpoint.
pub fn bang_pink() -> ColorU {
    ColorU::new(0xff, 0x2d, 0x78, 0xff)
}

/// Bang brand blue (`#4d7cff`), the gradient's bottom-right / cool endpoint.
pub fn bang_blue() -> ColorU {
    ColorU::new(0x4d, 0x7c, 0xff, 0xff)
}

pub fn bang_white() -> ColorU {
    ColorU::new(0xff, 0xff, 0xff, 0xff)
}

/// The Bang brand "app icon": a rounded-square tile with a diagonal pink -> blue
/// gradient (naturally passing through the brand purple), centered on a white
/// lightning mark. `size_px` is the square edge length in pixels.
pub fn bang_logo_mark(size_px: f32) -> Box<dyn Element> {
    let corner_radius = size_px * (REFERENCE_CORNER_RADIUS / REFERENCE_SIZE);
    let icon_size = size_px * (REFERENCE_ICON_SIZE / REFERENCE_SIZE);

    let lightning =
        ConstrainedBox::new(Icon::Lightning.to_warpui_icon(bang_white().into()).finish())
            .with_width(icon_size)
            .with_height(icon_size)
            .finish();

    ConstrainedBox::new(
        Container::new(Align::new(lightning).finish())
            .with_background_gradient(
                vec2f(0., 0.),
                vec2f(1., 1.),
                Gradient {
                    start: bang_pink(),
                    end: bang_blue(),
                },
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(corner_radius)))
            .finish(),
    )
    .with_width(size_px)
    .with_height(size_px)
    .finish()
}

/// Accent button theme carrying the Bang brand: a pink -> blue horizontal
/// gradient fill with white text/icon, giving a primary CTA the brand look
/// without restyling the rest of the slide.
pub struct BangButtonTheme;

impl button::themes::Theme for BangButtonTheme {
    fn background(&self, button_state: button::State, _appearance: &Appearance) -> Option<Fill> {
        let (pink, blue) = match button_state {
            button::State::Default => (bang_pink(), bang_blue()),
            // Dim slightly on hover / press for interaction feedback.
            button::State::Hovered => (
                ColorU::new(0xff, 0x2d, 0x78, 0xe6),
                ColorU::new(0x4d, 0x7c, 0xff, 0xe6),
            ),
            button::State::Pressed => (
                ColorU::new(0xff, 0x2d, 0x78, 0xcc),
                ColorU::new(0x4d, 0x7c, 0xff, 0xcc),
            ),
        };
        Some(Fill::HorizontalGradient(HorizontalGradient::new(
            pink, blue,
        )))
    }

    fn text_color(&self, _background: Option<Fill>, _appearance: &Appearance) -> ColorU {
        bang_white()
    }
}
