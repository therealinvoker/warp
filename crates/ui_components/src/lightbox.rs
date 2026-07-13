use std::sync::Arc;

use pathfinder_geometry::vector::{Vector2F, vec2f};
use warp_core::ui::Icon;
use warp_core::ui::appearance::Appearance;
use warpui_core::assets::asset_cache::AssetSource;
use warpui_core::elements::{
    CacheOption, Dismiss, DispatchEventResult, EventHandler, Image, Shrinkable,
};
use warpui_core::keymap::Keystroke;
use warpui_core::prelude::stack::*;
use warpui_core::prelude::*;

use crate::{Component, Options as _, button};

/// Padding between the scrim edge and the image.
const SCRIM_PADDING: f32 = 48.;

/// Spacing between the image/loading area and the description text.
const DESCRIPTION_SPACING: f32 = 12.;
const LIGHTBOX_TEXT_SIZE_DELTA: f32 = 4.;

/// Semi-transparent black background color for the scrim.
fn scrim_color() -> ColorU {
    ColorU::new(0, 0, 0, 230)
}

/// The loading state of a lightbox image.
#[derive(Clone, Debug)]
pub enum LightboxImageSource {
    /// The image metadata is still being fetched.
    Loading,
    /// The image source has been resolved.
    /// Note: the actual image bytes may still be loading via the `AssetCache`.
    Resolved { asset_source: AssetSource },
}

/// A single image entry in the lightbox.
#[derive(Clone, Debug)]
pub struct LightboxImage {
    /// The loading/loaded state of this image.
    pub source: LightboxImageSource,
    /// Optional description displayed below the image.
    pub description: Option<String>,
}

/// Direction for navigating between images.
#[derive(Clone, Copy, Debug)]
pub enum NavigationDirection {
    Previous,
    Next,
}

/// A handler invoked when the user navigates between images.
pub type NavigateHandler = Arc<dyn Fn(NavigationDirection, &mut EventContext, &AppContext)>;

/// A lightbox component for displaying images in a full-window overlay.
///
/// The lightbox displays one or more images centered on screen with a semi-transparent scrim
/// background. It supports navigating between images via arrow buttons and can be dismissed by
/// clicking outside the image, clicking the close button, or pressing Escape.
#[derive(Default)]
pub struct Lightbox {
    close_button: button::Button,
    prev_button: button::Button,
    next_button: button::Button,
    copy_button: button::Button,
    download_button: button::Button,
}

pub struct Params<'a> {
    /// The list of images to display.
    pub images: &'a [LightboxImage],

    /// The index of the currently displayed image.
    pub current_index: usize,

    /// Handler to invoke when the lightbox is dismissed.
    pub on_dismiss: DismissHandler,

    /// The native pixel dimensions of the currently displayed image, if known.
    /// When `Some`, the image is fully loaded and the lightbox renders it with a
    /// `ConstrainedBox` plus description. When `None`, the lightbox shows a loading
    /// indicator instead.
    pub current_image_native_size: Option<Vector2F>,

    /// Optional configuration for the lightbox.
    pub options: Options,
}

impl crate::Params for Params<'_> {
    type Options<'a> = Options;
}

/// A function that handles dismiss events.
pub type DismissHandler = Arc<dyn Fn(&mut EventContext, &AppContext)>;

pub struct Options {
    /// Optional keystroke associated with the dismiss action. This will be rendered alongside
    /// the dismiss button in the dialog, but the caller is responsible for adding a keybinding.
    pub dismiss_keystroke: Option<Keystroke>,

    /// Handler to invoke when the user navigates between images.
    /// If `None`, navigation buttons are not shown.
    pub on_navigate: Option<NavigateHandler>,

    /// Handler to invoke when the user copies the current image.
    /// If `None`, the "Copy Image" button is not shown.
    pub on_copy: Option<DismissHandler>,

    /// Handler to invoke when the user downloads the current image.
    /// If `None`, the "Download Image" button is not shown.
    pub on_download: Option<DismissHandler>,

    /// When true, the copy control renders in its "Copied!" confirmation state
    /// (check icon + "Copied!" label) instead of the default "Copy Image". The
    /// caller drives this flag on/off around a copy to flash feedback.
    pub copied: bool,
}

impl crate::Options for Options {
    fn default(_appearance: &Appearance) -> Self {
        Self {
            dismiss_keystroke: None,
            on_navigate: None,
            on_copy: None,
            on_download: None,
            copied: false,
        }
    }
}

impl Component for Lightbox {
    type Params<'a> = Params<'a>;

    fn render<'a>(&self, appearance: &Appearance, params: Self::Params<'a>) -> Box<dyn Element> {
        let on_dismiss_for_button = params.on_dismiss.clone();
        let on_dismiss = params.on_dismiss;
        let image_count = params.images.len();
        let current_index = params.current_index;
        // Whether to render the copy control in its transient "Copied!" state.
        let copied = params.options.copied;

        // Extract current image data via direct indexing.
        let current_image = params.images.get(current_index);
        let current_source = current_image.map(|img| &img.source);
        let current_description = current_image.and_then(|img| img.description.clone());
        let text_size = lightbox_text_size(appearance);

        // Copy and Download controls, rendered in the footer below the image.
        // They are only shown when the caller provides handlers and there is a
        // resolved image to act on.
        let has_current_image =
            matches!(current_source, Some(LightboxImageSource::Resolved { .. }));

        let copy_button = params
            .options
            .on_copy
            .filter(|_| has_current_image)
            .map(|on_copy| {
                self.copy_button.render(
                    appearance,
                    button::Params {
                        content: if copied {
                            button::Content::IconAndLabel(Icon::Check, "Copied!".into())
                        } else {
                            button::Content::IconAndLabel(Icon::Copy, "Copy Image".into())
                        },
                        theme: &ButtonTheme,
                        options: button::Options {
                            size: button::Size::Small,
                            on_click: Some(Box::new(move |ctx, app, _| {
                                on_copy(ctx, app);
                            })),
                            ..button::Options::default(appearance)
                        },
                    },
                )
            });

        let download_button = params
            .options
            .on_download
            .filter(|_| has_current_image)
            .map(|on_download| {
                self.download_button.render(
                    appearance,
                    button::Params {
                        content: button::Content::IconAndLabel(
                            Icon::Download,
                            "Download Image".into(),
                        ),
                        theme: &ButtonTheme,
                        options: button::Options {
                            size: button::Size::Small,
                            on_click: Some(Box::new(move |ctx, app, _| {
                                on_download(ctx, app);
                            })),
                            ..button::Options::default(appearance)
                        },
                    },
                )
            });

        // Close button in the top-right corner.
        let close_button = self.close_button.render(
            appearance,
            button::Params {
                content: button::Content::Icon(Icon::X),
                theme: &ButtonTheme,
                options: button::Options {
                    size: button::Size::Small,
                    on_click: Some(Box::new(move |ctx, app, _| {
                        on_dismiss_for_button(ctx, app);
                    })),
                    keystroke: params.options.dismiss_keystroke,
                    ..button::Options::default(appearance)
                },
            },
        );

        // Build the central content based on the image source and whether the
        // native size is known (i.e. the image data has been loaded).
        let central_content: Box<dyn Element> =
            match (current_source, params.current_image_native_size) {
                // Image source resolved AND native size known → render the image.
                (Some(LightboxImageSource::Resolved { asset_source }), Some(native_size)) => {
                    let image = ConstrainedBox::new(
                        Image::new(asset_source.clone(), CacheOption::Original)
                            .contain()
                            .layout_using_paint_bounds()
                            .before_load(Align::new(loading_element(appearance)).finish())
                            .finish(),
                    )
                    .with_max_width(native_size.x())
                    .with_max_height(native_size.y())
                    .finish();

                    EventHandler::new(image)
                        .on_left_mouse_down(|_, _, _| DispatchEventResult::StopPropagation)
                        .finish()
                }
                // No images provided at all.
                _ if image_count == 0 => {
                    Text::new("No images", appearance.ui_font_family(), text_size)
                        .with_color(ColorU::white())
                        .finish()
                }
                // Still loading (either metadata or image bytes).
                _ => loading_element(appearance),
            };

        // Footer shown below the image once it is fully loaded (native size
        // known): an optional description and the copy/download action buttons.
        let image_loaded = params.current_image_native_size.is_some();
        let mut footer_children: Vec<Box<dyn Element>> = Vec::new();
        if image_loaded {
            if let Some(description) = current_description {
                footer_children.push(
                    Text::new(description, appearance.ui_font_family(), text_size)
                        .with_color(ColorU::white())
                        .finish(),
                );
            }
            if copy_button.is_some() || download_button.is_some() {
                let mut actions = Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(8.);
                if let Some(copy_button) = copy_button {
                    actions = actions.with_child(copy_button);
                }
                if let Some(download_button) = download_button {
                    actions = actions.with_child(download_button);
                }
                footer_children.push(actions.finish());
            }
        }

        let content_with_footer = if footer_children.is_empty() {
            central_content
        } else {
            let mut column = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(DESCRIPTION_SPACING)
                .with_child(Shrinkable::new(1.0, central_content).finish());
            for child in footer_children {
                column = column.with_child(child);
            }
            column.finish()
        };

        let centered_content = Align::new(content_with_footer).finish();

        let scrim = Container::new(
            Dismiss::new(centered_content)
                .prevent_interaction_with_other_elements()
                .on_dismiss(move |ctx, app| on_dismiss(ctx, app))
                .finish(),
        )
        .with_background_color(scrim_color())
        .with_uniform_padding(SCRIM_PADDING)
        .finish();

        // Stack the scrim, close button, and optional navigation arrows. The
        // copy/download controls live in the footer below the image.
        let mut content = Stack::new().with_child(scrim);
        content.add_positioned_child(
            close_button,
            OffsetPositioning::offset_from_parent(
                vec2f(-12., 12.),
                ParentOffsetBounds::Unbounded,
                ParentAnchor::TopRight,
                ChildAnchor::TopRight,
            ),
        );

        // Navigation arrows (only shown when there are multiple images).
        if image_count > 1
            && let Some(on_navigate) = params.options.on_navigate
        {
            // Previous button (hidden on first image).
            if current_index > 0 {
                let on_nav = on_navigate.clone();
                let prev_button = self.prev_button.render(
                    appearance,
                    button::Params {
                        content: button::Content::Icon(Icon::ChevronLeft),
                        theme: &ButtonTheme,
                        options: button::Options {
                            size: button::Size::Small,
                            on_click: Some(Box::new(move |ctx, app, _| {
                                on_nav(NavigationDirection::Previous, ctx, app);
                            })),
                            ..button::Options::default(appearance)
                        },
                    },
                );
                content.add_positioned_child(
                    prev_button,
                    OffsetPositioning::offset_from_parent(
                        vec2f(12., 0.),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::MiddleLeft,
                        ChildAnchor::MiddleLeft,
                    ),
                );
            }

            // Next button (hidden on last image).
            if current_index < image_count - 1 {
                let on_nav = on_navigate;
                let next_button = self.next_button.render(
                    appearance,
                    button::Params {
                        content: button::Content::Icon(Icon::ChevronRight),
                        theme: &ButtonTheme,
                        options: button::Options {
                            size: button::Size::Small,
                            on_click: Some(Box::new(move |ctx, app, _| {
                                on_nav(NavigationDirection::Next, ctx, app);
                            })),
                            ..button::Options::default(appearance)
                        },
                    },
                );
                content.add_positioned_child(
                    next_button,
                    OffsetPositioning::offset_from_parent(
                        vec2f(-12., 0.),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::MiddleRight,
                        ChildAnchor::MiddleRight,
                    ),
                );
            }
        }

        content.finish()
    }
}

/// Builds the shared "Loading..." text element used in both the `Loading` state
/// and as the `before_load` fallback while the `AssetCache` fetches image bytes.
fn loading_element(appearance: &Appearance) -> Box<dyn Element> {
    Text::new(
        "Loading...",
        appearance.ui_font_family(),
        lightbox_text_size(appearance),
    )
    .with_color(ColorU::white())
    .finish()
}

fn lightbox_text_size(appearance: &Appearance) -> f32 {
    appearance.ui_font_size() + LIGHTBOX_TEXT_SIZE_DELTA
}

/// A custom button theme for lightbox buttons to force colors to match
/// a Dark theme button, as these buttons always appear on top of a near-black
/// scrim, independent of application theme.
struct ButtonTheme;

impl button::Theme for ButtonTheme {
    fn background(
        &self,
        button_state: button::State,
        _appearance: &Appearance,
    ) -> Option<warp_core::ui::theme::Fill> {
        match button_state {
            button::State::Default => None,
            button::State::Hovered => Some(warp_core::ui::theme::Fill::white().with_opacity(10)),
            button::State::Pressed => Some(warp_core::ui::theme::Fill::white().with_opacity(15)),
        }
    }

    fn text_color(
        &self,
        _background: Option<warp_core::ui::theme::Fill>,
        _appearance: &Appearance,
    ) -> ColorU {
        ColorU::new(255, 255, 255, 255)
    }

    fn border(&self, _appearance: &Appearance) -> Option<ColorU> {
        Some(ColorU::new(51, 51, 51, 255))
    }

    fn keyboard_shortcut_background(&self, _appearance: &Appearance) -> Option<ColorU> {
        Some(ColorU::new(38, 38, 38, 255))
    }
}
