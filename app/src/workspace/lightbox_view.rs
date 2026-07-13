use std::sync::Arc;
use std::time::Duration;

pub use lightbox::LightboxImage;
use pathfinder_geometry::vector::Vector2F;
use ui_components::{lightbox, Component as _};
use warpui::assets::asset_cache::{AssetCache, AssetSource, AssetState};
use warpui::clipboard::{ClipboardContent, ImageData};
use warpui::image_cache::ImageType;
use warpui::keymap::{FixedBinding, Keystroke};
use warpui::platform::SaveFilePickerConfiguration;
use warpui::prelude::*;
use warpui::r#async::Timer;
use warpui::{AppContext, BlurContext, Element, Entity, SingletonEntity, View, ViewContext};

use crate::appearance::Appearance;

/// Default filename used when copying or downloading a lightbox image, since
/// the lightbox does not always know the original file name of its source.
const DEFAULT_IMAGE_FILENAME: &str = "image.png";

/// How long the copy control shows its "Copied!" confirmation after a copy
/// (auto-copy on open, or a manual click) before reverting to "Copy Image".
const COPY_FEEDBACK_DURATION: Duration = Duration::from_millis(1500);

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;
    let view_id = id!(LightboxView::ui_name());
    app.register_fixed_bindings([
        FixedBinding::new("escape", LightboxViewAction::Dismiss, view_id.clone()),
        FixedBinding::new(
            "left",
            LightboxViewAction::NavigatePrevious,
            view_id.clone(),
        ),
        FixedBinding::new("right", LightboxViewAction::NavigateNext, view_id),
    ]);
}

/// Parameters needed to open a lightbox.
#[derive(Clone, Debug)]
pub struct LightboxParams {
    /// The images to display in the lightbox.
    pub images: Vec<LightboxImage>,
    /// The index of the image to display initially.
    pub initial_index: usize,
    /// When true, the initially shown image is copied to the clipboard as soon
    /// as it finishes loading, and the copy control flashes "Copied!".
    pub auto_copy: bool,
}

/// Events emitted by the `LightboxView` to its parent.
pub enum LightboxViewEvent {
    /// The user explicitly dismissed the lightbox (Escape, close button, or scrim click).
    Close,
    /// Focus left the lightbox subtree (e.g. the user switched tabs).
    FocusLost,
}

impl Entity for LightboxView {
    type Event = LightboxViewEvent;
}

/// Actions dispatched within the `LightboxView`.
#[derive(Debug)]
pub enum LightboxViewAction {
    /// Dismiss the lightbox (triggered by clicking outside, close button, or Escape).
    Dismiss,
    /// Navigate to the previous image.
    NavigatePrevious,
    /// Navigate to the next image.
    NavigateNext,
    /// Copy the current image to the system clipboard.
    CopyImage,
    /// Save the current image to disk via the native save dialog.
    DownloadImage,
}

/// A view that renders a full-window lightbox overlay.
pub struct LightboxView {
    params: LightboxParams,
    current_index: usize,
    lightbox: lightbox::Lightbox,
    /// Whether the auto-copy-on-open has already fired for the current open, so
    /// it happens at most once even as multiple asset loads complete.
    has_auto_copied: bool,
    /// Whether the copy control is currently showing its "Copied!" state.
    show_copied_feedback: bool,
    /// Bumped every time the "Copied!" feedback starts, so a stale revert timer
    /// from an earlier copy doesn't clear a newer one.
    copy_feedback_epoch: usize,
}

impl LightboxView {
    pub fn new(params: LightboxParams, ctx: &mut ViewContext<Self>) -> Self {
        let initial_index = params
            .initial_index
            .min(params.images.len().saturating_sub(1));
        let mut view = Self {
            params,
            current_index: initial_index,
            lightbox: lightbox::Lightbox::default(),
            has_auto_copied: false,
            show_copied_feedback: false,
            copy_feedback_epoch: 0,
        };
        view.start_asset_loads(ctx);
        view.try_auto_copy(ctx);
        view
    }

    /// Replace the images and navigate to the given initial index.
    pub fn update_params(&mut self, params: LightboxParams, ctx: &mut ViewContext<Self>) {
        let initial_index = params
            .initial_index
            .min(params.images.len().saturating_sub(1));
        self.params = params;
        self.current_index = initial_index;
        // A fresh open resets auto-copy and cancels any in-flight "Copied!"
        // revert timer from the previous open.
        self.has_auto_copied = false;
        self.show_copied_feedback = false;
        self.copy_feedback_epoch = self.copy_feedback_epoch.wrapping_add(1);
        self.start_asset_loads(ctx);
        self.try_auto_copy(ctx);
    }

    /// Copy the current image to the clipboard once its bytes are available,
    /// then flash the "Copied!" confirmation. No-op when auto-copy is disabled,
    /// it has already fired, or the image has not finished loading yet (in which
    /// case the asset-load callback retries once bytes arrive).
    fn try_auto_copy(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.params.auto_copy || self.has_auto_copied {
            return;
        }
        if self.write_current_image_to_clipboard(ctx) {
            self.has_auto_copied = true;
            self.begin_copied_feedback(ctx);
        }
    }

    /// Write the current image (re-encoded to PNG) to the system clipboard.
    /// Returns whether an image was actually written (false while loading or for
    /// non-bitmap sources).
    fn write_current_image_to_clipboard(&self, ctx: &mut ViewContext<Self>) -> bool {
        let Some(png_bytes) = self.current_image_png_bytes(ctx) else {
            return false;
        };
        ctx.clipboard().write(ClipboardContent {
            images: Some(vec![ImageData {
                data: png_bytes,
                mime_type: "image/png".to_string(),
                filename: Some(DEFAULT_IMAGE_FILENAME.to_string()),
            }]),
            ..Default::default()
        });
        true
    }

    /// Show the "Copied!" state on the copy control and schedule its revert.
    fn begin_copied_feedback(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_copied_feedback = true;
        self.copy_feedback_epoch = self.copy_feedback_epoch.wrapping_add(1);
        let epoch = self.copy_feedback_epoch;
        ctx.notify();
        ctx.spawn(
            async move {
                Timer::after(COPY_FEEDBACK_DURATION).await;
                epoch
            },
            Self::clear_copied_feedback,
        );
    }

    /// Revert the copy control from "Copied!" back to "Copy Image", unless a
    /// newer copy has since restarted the feedback (epoch mismatch).
    fn clear_copied_feedback(&mut self, epoch: usize, ctx: &mut ViewContext<Self>) {
        if epoch == self.copy_feedback_epoch {
            self.show_copied_feedback = false;
            ctx.notify();
        }
    }

    /// Update a single image at the given index without replacing the full list.
    pub fn update_image_at(
        &mut self,
        index: usize,
        image: LightboxImage,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(slot) = self.params.images.get_mut(index) {
            if let lightbox::LightboxImageSource::Resolved { ref asset_source } = image.source {
                Self::start_asset_load(asset_source, ctx);
            }
            *slot = image;
        }
    }

    /// Kick off asset loads for all `Resolved` images and schedule re-renders.
    fn start_asset_loads(&self, ctx: &mut ViewContext<Self>) {
        for img in &self.params.images {
            if let lightbox::LightboxImageSource::Resolved { ref asset_source } = img.source {
                Self::start_asset_load(asset_source, ctx);
            }
        }
    }

    /// Encode the currently displayed image to PNG bytes, if it is resolved and
    /// its bytes have finished loading. Returns `None` while loading, for
    /// non-bitmap sources (e.g. SVG), or if encoding fails. The decoded image is
    /// re-encoded to PNG so copy/download works uniformly regardless of the
    /// original source (raw bytes, local file, or URL).
    fn current_image_png_bytes(&self, app: &AppContext) -> Option<Vec<u8>> {
        let asset_source = match &self.params.images.get(self.current_index)?.source {
            lightbox::LightboxImageSource::Resolved { asset_source } => asset_source.clone(),
            lightbox::LightboxImageSource::Loading => return None,
        };
        match AssetCache::as_ref(app).load_asset::<ImageType>(asset_source) {
            AssetState::Loaded { data } => encode_image_to_png(&data),
            AssetState::Loading { .. } | AssetState::Evicted | AssetState::FailedToLoad(_) => None,
        }
    }

    /// Eagerly load a single asset and schedule a `ctx.notify()` when the fetch
    /// completes so the lightbox re-renders with the loaded image.
    fn start_asset_load(asset_source: &AssetSource, ctx: &mut ViewContext<Self>) {
        let asset_cache = AssetCache::as_ref(ctx);
        if let AssetState::Loading { handle } =
            asset_cache.load_asset::<ImageType>(asset_source.clone())
        {
            if let Some(future) = handle.when_loaded(asset_cache) {
                ctx.spawn(future, |me, (), ctx| {
                    // Now that bytes have arrived, auto-copy may be able to run
                    // (it no-ops if it already fired or isn't the current image).
                    me.try_auto_copy(ctx);
                    ctx.notify();
                });
            }
        }
    }
}

impl View for LightboxView {
    fn ui_name() -> &'static str {
        "LightboxView"
    }

    fn on_blur(&mut self, _blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        // Only dismiss if focus has left the entire lightbox subtree.
        if !ctx.is_self_or_child_focused() {
            ctx.emit(LightboxViewEvent::FocusLost);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        // Determine the native pixel size of the current image by querying the
        // asset cache. This will be `Some` once the image bytes have been fully
        // loaded and decoded.
        let current_image_native_size =
            self.params
                .images
                .get(self.current_index)
                .and_then(|img| match &img.source {
                    lightbox::LightboxImageSource::Resolved { asset_source } => {
                        let asset_cache = AssetCache::as_ref(app);
                        match asset_cache.load_asset::<ImageType>(asset_source.clone()) {
                            AssetState::Loaded { data } => data
                                .image_size()
                                .map(|size| Vector2F::new(size.x() as f32, size.y() as f32)),
                            _ => None,
                        }
                    }
                    lightbox::LightboxImageSource::Loading => None,
                });

        self.lightbox.render(
            appearance,
            lightbox::Params {
                images: &self.params.images,
                current_index: self.current_index,
                on_dismiss: Arc::new(|ctx, _| {
                    ctx.dispatch_typed_action(LightboxViewAction::Dismiss);
                }),
                current_image_native_size,
                options: lightbox::Options {
                    dismiss_keystroke: Keystroke::parse("escape").ok(),
                    on_navigate: Some(Arc::new(|direction, ctx, _| match direction {
                        lightbox::NavigationDirection::Previous => {
                            ctx.dispatch_typed_action(LightboxViewAction::NavigatePrevious);
                        }
                        lightbox::NavigationDirection::Next => {
                            ctx.dispatch_typed_action(LightboxViewAction::NavigateNext);
                        }
                    })),
                    on_copy: Some(Arc::new(|ctx, _| {
                        ctx.dispatch_typed_action(LightboxViewAction::CopyImage);
                    })),
                    on_download: Some(Arc::new(|ctx, _| {
                        ctx.dispatch_typed_action(LightboxViewAction::DownloadImage);
                    })),
                    copied: self.show_copied_feedback,
                },
            },
        )
    }
}

impl TypedActionView for LightboxView {
    type Action = LightboxViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            LightboxViewAction::Dismiss => {
                ctx.emit(LightboxViewEvent::Close);
            }
            LightboxViewAction::NavigatePrevious => {
                if self.current_index > 0 {
                    self.current_index -= 1;
                    ctx.notify();
                }
            }
            LightboxViewAction::NavigateNext => {
                if self.current_index + 1 < self.params.images.len() {
                    self.current_index += 1;
                    ctx.notify();
                }
            }
            LightboxViewAction::CopyImage => {
                // Manual copies flash the same "Copied!" confirmation as auto-copy.
                if self.write_current_image_to_clipboard(ctx) {
                    self.begin_copied_feedback(ctx);
                }
            }
            LightboxViewAction::DownloadImage => {
                let Some(png_bytes) = self.current_image_png_bytes(ctx) else {
                    return;
                };
                let config = SaveFilePickerConfiguration::new()
                    .with_default_filename(DEFAULT_IMAGE_FILENAME.to_string());
                ctx.open_save_file_picker(
                    move |path_opt, _me, ctx| {
                        let Some(path) = path_opt else {
                            return;
                        };
                        ctx.spawn(
                            async move { async_fs::write(&path, &png_bytes).await },
                            |_me, result, _ctx| {
                                if let Err(e) = result {
                                    log::warn!("Failed to save image from lightbox: {e}");
                                }
                            },
                        );
                    },
                    config,
                );
            }
        }
    }
}

/// Re-encode a decoded image to PNG bytes. Bitmap frames are converted from
/// their in-memory RGBA representation; animated images use their first frame.
/// Returns `None` for vector or unrecognized sources, which we cannot copy or
/// download as a raster image.
fn encode_image_to_png(image_type: &ImageType) -> Option<Vec<u8>> {
    let static_image = match image_type {
        ImageType::StaticBitmap { image } => image.clone(),
        ImageType::AnimatedBitmap { image } => image.frames.first()?.image.clone(),
        ImageType::Svg { .. } | ImageType::Unrecognized => return None,
    };

    let rgba = image::RgbaImage::from_raw(
        static_image.width(),
        static_image.height(),
        static_image.rgba_bytes().to_vec(),
    )?;

    let mut png_bytes = Vec::new();
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .ok()?;
    Some(png_bytes)
}
