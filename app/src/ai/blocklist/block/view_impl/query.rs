//! Renders the user query portion of the AI block, if there is one.
//!
//! Queries are not rendered in blocks corresponding to requested command or requested action responses.

use warp_core::features::FeatureFlag;
use warp_core::ui::color::contrast::relative_luminance;
use warp_core::ui::theme::color::internal_colors;
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DispatchEventResult,
    EventHandler, Expanded, Flex, Image as WarpImage, MainAxisAlignment, MainAxisSize,
    ParentElement, Radius, Wrap,
};
use warpui::fonts::{Properties, Style, Weight};
use warpui::image_cache::CacheOption;
use warpui::ui_components::chip::Chip;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, SingletonEntity};
use warpui_core::color::ColorU;

use super::common::{render_query_text, FindContext};
use super::{USER_BUBBLE_HORIZONTAL_PADDING, USER_BUBBLE_VERTICAL_PADDING};
use crate::ai::blocklist::block::view_impl::common::UserQueryProps;
use crate::ai::blocklist::block::{AIBlockAction, DetectedLinksState, SecretRedactionState};
use crate::ai::blocklist::AttachmentType;
use crate::appearance::Appearance;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;

/// Data required to render the AI block query component.
#[derive(Copy, Clone, Debug)]
pub(super) struct Props<'a> {
    pub(super) query_and_index: Option<(&'a str, usize)>,
    pub(super) query_prefix_highlight_len: Option<usize>,
    pub(super) detected_links_state: &'a DetectedLinksState,
    pub(super) secret_redaction_state: &'a SecretRedactionState,
    pub(super) is_selecting_text: bool,
    pub(super) is_ai_input_enabled: bool,
    pub(super) attachments: &'a [(AttachmentType, String)],
    /// Staged inline thumbnails for the image attachments, in the same order as
    /// the `AttachmentType::Image` entries in `attachments`.
    pub(super) image_thumbnails: &'a [(AssetSource, f32)],
    pub(super) find_context: Option<FindContext<'a>>,
}

pub(super) fn maybe_render(
    props: Props,
    overflow_menu: Option<Box<dyn Element>>,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    props.query_and_index.map(|(query, input_index)| {
        render_query(
            query,
            props.detected_links_state,
            props.secret_redaction_state,
            input_index,
            props.query_prefix_highlight_len,
            props.is_selecting_text,
            props.is_ai_input_enabled,
            props.attachments,
            props.image_thumbnails,
            props.find_context,
            overflow_menu,
            app,
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_query(
    query: &str,
    detected_links_state: &DetectedLinksState,
    secret_redaction_state: &SecretRedactionState,
    input_index: usize,
    query_prefix_highlight_len: Option<usize>,
    is_selecting: bool,
    is_ai_input_enabled: bool,
    attachments: &[(AttachmentType, String)],
    image_thumbnails: &[(AssetSource, f32)],
    find_context: Option<FindContext>,
    overflow_menu: Option<Box<dyn Element>>,
    app: &AppContext,
) -> Box<dyn Element> {
    let properties = Properties {
        style: Style::Normal,
        weight: Weight::Normal,
    };
    // The query already includes the /plan prefix when in plan mode via display_user_query()
    let text_element = render_query_text(
        UserQueryProps {
            text: query.to_owned(),
            query_prefix_highlight_len,
            detected_links_state,
            secret_redaction_state,
            input_index,
            is_selecting,
            is_ai_input_enabled,
            find_context,
            font_properties: &properties,
        },
        app,
    );

    let appearance = Appearance::as_ref(app);
    // The user query no longer renders a leading avatar; the query text sits flush at
    // the block's left content padding so the whole conversation column (query + agent
    // output) has symmetric left/right padding.
    let mut query = Flex::column().with_child(text_element.finish());

    if FeatureFlag::ImageAsContext.is_enabled() {
        query = query.with_child(render_attachments(
            attachments,
            image_thumbnails,
            appearance,
        ));
    }

    // When present, the `⋮` overflow menu lives INSIDE the bubble, right-aligned, so
    // the grey card visually contains the actions menu instead of it floating outside
    // to the right. The query text takes the remaining width via `Expanded`, pushing
    // the menu to the bubble's right edge.
    let bubble_content: Box<dyn Element> = match overflow_menu {
        Some(menu) => Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Expanded::new(1., query.finish()).finish())
            .with_child(menu)
            .finish(),
        None => query.finish(),
    };

    // Wrap the user comment in a rounded, subtly-shaded bubble (a chat "bubble") so
    // user messages read as visually distinct from agent responses. Uses the subtlest
    // surface token (`neutral_1`), lightened by 8% in dark mode so the card reads a
    // touch brighter than the conversation background, with a thin outline one step
    // lighter than the fill so the card edge is just visible.
    Container::new(bubble_content)
        .with_background_color(user_bubble_background(appearance))
        .with_border(Border::all(1.).with_border_color(user_bubble_border(appearance)))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
        .with_horizontal_padding(USER_BUBBLE_HORIZONTAL_PADDING)
        .with_vertical_padding(USER_BUBBLE_VERTICAL_PADDING)
        .finish()
}

/// Fill for the user-comment bubble: the subtle `neutral_1` surface, lightened
/// toward white by 8% in dark mode so the card sits a touch brighter than the
/// conversation background without reading as a bright box. Light themes keep the
/// unmodified surface to avoid washing the card out to near-white.
pub(super) fn user_bubble_background(appearance: &Appearance) -> ColorU {
    let theme = appearance.theme();
    let base = internal_colors::neutral_1(theme);
    let is_dark_background = relative_luminance(theme.background().into_solid()) < 0.2;
    if is_dark_background {
        lighten_toward_white(base, 0.08)
    } else {
        base
    }
}

/// Thin outline for the user-comment bubble: `neutral_1` lightened 10% toward
/// white, i.e. one step lighter than the 8% fill so the card edge is subtly
/// visible against both the card and the conversation background.
pub(super) fn user_bubble_border(appearance: &Appearance) -> ColorU {
    lighten_toward_white(internal_colors::neutral_1(appearance.theme()), 0.10)
}

/// Lightens `color` toward white by `factor` (0.0–1.0). Mirrors the blend used by
/// [`warp_core::ui::color::lighten`] but with a caller-chosen factor.
fn lighten_toward_white(color: ColorU, factor: f32) -> ColorU {
    let lighten_channel = |channel: u8| channel + (((255 - channel) as f32) * factor).round() as u8;
    ColorU::new(
        lighten_channel(color.r),
        lighten_channel(color.g),
        lighten_channel(color.b),
        color.a,
    )
}

fn render_attachments(
    attachments: &[(AttachmentType, String)],
    image_thumbnails: &[(AssetSource, f32)],
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut image_index = 0;
    let chips = attachments.iter().map(|(attachment_type, file_name)| {
        if matches!(attachment_type, AttachmentType::Image) {
            let clicked_image_index = image_index;
            let thumbnail = image_thumbnails.get(image_index).cloned();
            image_index += 1;
            // Prefer an inline thumbnail of the actual image; fall back to the
            // icon + filename chip only if the thumbnail couldn't be staged.
            let content = match thumbnail {
                Some((asset_source, aspect)) => render_image_thumbnail(asset_source, aspect),
                None => render_filename_chip(AttachmentType::Image, file_name, appearance),
            };
            return EventHandler::new(content)
                .on_left_mouse_down(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AIBlockAction::OpenSubmittedAttachmentLightbox {
                        image_index: clicked_image_index,
                    });
                    DispatchEventResult::StopPropagation
                })
                .finish();
        }

        render_filename_chip(*attachment_type, file_name, appearance)
    });

    if attachments.is_empty() {
        Flex::row().finish()
    } else {
        let wrapping_section = Wrap::row()
            .with_run_spacing(8.)
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Min)
            .with_children(chips)
            .finish();
        Container::new(wrapping_section)
            .with_padding_top(7.)
            .finish()
    }
}

/// Renders an inline thumbnail of a submitted image attachment. The box is sized
/// to the image's exact aspect (captured at staging time) so `.contain()` fills
/// it with no letterboxing.
fn render_image_thumbnail(asset_source: AssetSource, aspect: f32) -> Box<dyn Element> {
    const ATTACHMENT_THUMBNAIL_HEIGHT: f32 = 36.;
    const ATTACHMENT_THUMBNAIL_MAX_WIDTH: f32 = 160.;
    let aspect = aspect.max(0.01);
    let (width, height) = if aspect >= ATTACHMENT_THUMBNAIL_MAX_WIDTH / ATTACHMENT_THUMBNAIL_HEIGHT
    {
        (
            ATTACHMENT_THUMBNAIL_MAX_WIDTH,
            ATTACHMENT_THUMBNAIL_MAX_WIDTH / aspect,
        )
    } else {
        (
            ATTACHMENT_THUMBNAIL_HEIGHT * aspect,
            ATTACHMENT_THUMBNAIL_HEIGHT,
        )
    };
    let image = WarpImage::new(asset_source, CacheOption::BySize)
        .contain()
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
    Container::new(
        ConstrainedBox::new(Box::new(image))
            .with_width(width)
            .with_height(height)
            .finish(),
    )
    .with_margin_right(6.)
    .finish()
}

fn render_filename_chip(
    attachment_type: AttachmentType,
    file_name: &str,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let icon = match attachment_type {
        AttachmentType::Image => Icon::Image,
        AttachmentType::File => Icon::File,
    };
    Chip::new(
        file_name.to_owned(),
        UiComponentStyles {
            margin: Some(Coords {
                top: 0.,
                bottom: 0.,
                left: 0.,
                right: 6.,
            }),
            font_family_id: Some(appearance.ui_font_family()),
            font_size: Some(appearance.monospace_font_size()),
            font_color: Some(blended_colors::text_sub(
                appearance.theme(),
                appearance.theme().background(),
            )),
            border_width: Some(1.),
            border_color: Some(internal_colors::neutral_4(appearance.theme()).into()),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(5.))),
            ..Default::default()
        },
    )
    .with_icon(icon.to_warpui_icon(
        blended_colors::text_sub(appearance.theme(), appearance.theme().background()).into(),
    ))
    .build()
    .finish()
}
