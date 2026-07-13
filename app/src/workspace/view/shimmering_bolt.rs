//! A lightning-bolt "agent is working" indicator with a spotlight/gleam that sweeps
//! across it.
//!
//! Repaints re-run `layout`/`paint` on the *cached* element tree — they do not rebuild the
//! view via `render` — so any time-based animation must be computed inside `paint`, exactly
//! like `ShimmeringTextElement` does. Values baked in at build time (e.g. an `Icon` with a
//! precomputed opacity) would simply be frozen. Each frame we draw the bolt dim, then draw a
//! brighter copy clipped to a narrow vertical band whose position advances with wall-clock
//! time, producing a highlight that sweeps left-to-right across the glyph.

use std::sync::LazyLock;
use std::time::Duration;

use instant::Instant;
use pathfinder_color::ColorU;
use warpui::assets::asset_cache::{AssetCache, AssetSource, AssetState};
use warpui::elements::{
    AfterLayoutContext, Element, EventContext, LayoutContext, PaintContext, Point, SizeConstraint,
};
use warpui::event::DispatchedEvent;
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::{vec2f, Vector2F};
use warpui::image_cache::{AnimatedImageBehavior, CacheOption, FitType, Image, ImageCache};
use warpui::{AppContext, ClipBounds, SingletonEntity as _};

/// Repaint cadence (~30fps) for the sweeping spotlight animation.
const REPAINT_INTERVAL: Duration = Duration::from_millis(33);

/// Duration of one full left-to-right sweep of the spotlight.
const SWEEP_PERIOD: Duration = Duration::from_millis(1100);

/// Width of the bright spotlight band as a fraction of the glyph width.
const BAND_FRACTION: f32 = 0.5;

/// Opacity of the always-visible dim base glyph (outside the spotlight band).
const BASE_OPACITY: f32 = 0.4;

/// Shared epoch so every working row's spotlight sweeps in sync.
static SWEEP_EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Position of the spotlight band's center along the sweep, in `[0, 1]`.
fn sweep_progress() -> f32 {
    let period = SWEEP_PERIOD.as_secs_f32();
    let elapsed = SWEEP_EPOCH.elapsed().as_secs_f32();
    (elapsed / period).fract()
}

/// A monochrome bolt glyph with a spotlight sweeping across it. `base_color` tints the dim
/// resting glyph; `highlight_color` tints the bright band. Keep both neutral (black & white)
/// so the indicator reads as a gleam rather than a colored accent.
pub struct ShimmeringBoltElement {
    path: &'static str,
    base_color: ColorU,
    highlight_color: ColorU,
    size: Option<Vector2F>,
    origin: Option<Point>,
}

impl ShimmeringBoltElement {
    pub fn new(path: &'static str, base_color: ColorU, highlight_color: ColorU) -> Self {
        Self {
            path,
            base_color,
            highlight_color,
            size: None,
            origin: None,
        }
    }
}

impl Element for ShimmeringBoltElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        _: &mut LayoutContext,
        _: &AppContext,
    ) -> Vector2F {
        let size = constraint.max;
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _: &mut AfterLayoutContext, _: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        let Some(size) = self.size else {
            return;
        };
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));

        let bounds = (size * ctx.scene.scale_factor()).to_i32();
        if bounds.x() <= 0 || bounds.y() <= 0 {
            return;
        }

        let asset_cache = AssetCache::as_ref(app);
        let image = ImageCache::as_ref(app).image(
            AssetSource::Bundled { path: self.path },
            bounds,
            FitType::Contain,
            AnimatedImageBehavior::FullAnimation,
            CacheOption::BySize,
            ctx.max_texture_dimension_2d,
            asset_cache,
        );

        match image {
            AssetState::Loaded { data } => {
                let Image::Static(static_image) = data.as_ref() else {
                    return;
                };

                let logical_image_size = static_image.size().to_f32() / ctx.scene.scale_factor();
                let icon_origin = origin + ((size - logical_image_size) / 2.0);
                let icon_rect = RectF::new(icon_origin, logical_image_size);

                // Dim resting glyph, always visible.
                ctx.scene.draw_icon(
                    icon_rect,
                    static_image.clone(),
                    BASE_OPACITY,
                    self.base_color,
                );

                // Bright band sweeping left-to-right, clipped to a moving vertical stripe so
                // only the portion of the glyph under the "spotlight" brightens.
                let band_width = logical_image_size.x() * BAND_FRACTION;
                let travel = logical_image_size.x() + band_width;
                let band_center_x = icon_origin.x() - band_width / 2.0 + sweep_progress() * travel;
                let band_rect = RectF::new(
                    vec2f(band_center_x - band_width / 2.0, icon_origin.y()),
                    vec2f(band_width, logical_image_size.y()),
                );
                ctx.scene
                    .start_layer(ClipBounds::BoundedByActiveLayerAnd(band_rect));
                ctx.scene
                    .draw_icon(icon_rect, static_image.clone(), 1.0, self.highlight_color);
                ctx.scene.stop_layer();

                ctx.repaint_after(REPAINT_INTERVAL);
            }
            AssetState::Loading { handle } => ctx.repaint_after_load(handle),
            AssetState::Evicted | AssetState::FailedToLoad(_) => {}
        }
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        _: &DispatchedEvent,
        _: &mut EventContext,
        _: &AppContext,
    ) -> bool {
        false
    }
}
