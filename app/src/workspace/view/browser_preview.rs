//! In-app browser "Preview" tab for the tools drawer.
//!
//! Renders a URL bar + navigation controls above a content region that is a
//! placeholder for a native embedded web view (macOS `WKWebView`). The native
//! web view is a subview of the window content view and is composited *above*
//! the Metal-rendered UI, so the content region here only paints a placeholder
//! that is visible while the web view is hidden (before navigation, or on
//! platforms without embedded web view support).
//!
//! Geometry sync: the content region is wrapped in a [`SavePosition`] keyed by
//! [`BROWSER_PREVIEW_WEB_VIEW_ID`]. After each drawn frame we read that rect and
//! drive the native web view's frame/visibility to match.

use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;
use warpui::elements::{
    Align, Border, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element,
    Empty, Expanded, Flex, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement,
    Radius, SavePosition, Text,
};
use warpui::platform::Cursor;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};
use warpui_core::zoom::Scale;

use crate::appearance::Appearance;
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
    TextOptions,
};
use crate::ui_components::buttons::icon_button;
use crate::ui_components::icons::Icon;

/// Stable id used to both record the preview content region via [`SavePosition`]
/// and to key the per-window native web view.
pub const BROWSER_PREVIEW_WEB_VIEW_ID: &str = "workspace:browser_preview";

/// Height of the URL/navigation bar at the top of the preview tab.
const NAV_BAR_HEIGHT: f64 = 34.0;

/// Horizontal inset (in layout points) applied to the native web view frame on
/// both edges. The panel's resize dragbar is a ~5px hit region at the panel
/// edge; the native `WKWebView` is composited above the Metal layer and would
/// otherwise swallow mouse events over that region, making the panel almost
/// impossible to drag-resize. Insetting slightly more than the dragbar width
/// keeps the resize handle grabbable regardless of which side the panel (and
/// therefore its dragbar) is docked on.
const WEB_VIEW_RESIZE_MARGIN: f32 = 6.0;

#[derive(Clone, Debug)]
pub enum BrowserPreviewAction {
    /// Navigate to the URL currently entered in the URL bar.
    SubmitUrl,
    /// Reload the current page.
    Reload,
    /// Navigate back in history.
    Back,
    /// Navigate forward in history.
    Forward,
}

pub struct BrowserPreviewView {
    url_editor: ViewHandle<EditorView>,
    current_url: Option<String>,
    back_mouse_state: MouseStateHandle,
    forward_mouse_state: MouseStateHandle,
    reload_mouse_state: MouseStateHandle,
    go_mouse_state: MouseStateHandle,
    /// Sends a tick after each drawn frame so the async loop can re-sync the
    /// native web view geometry with the laid-out placeholder rect.
    frame_tick_tx: futures::channel::mpsc::UnboundedSender<()>,
    /// Whether a post-frame sync tick is already queued, to avoid arming more
    /// than one callback per frame.
    frame_sync_armed: bool,
}

impl BrowserPreviewView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let url_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let options = SingleLineEditorOptions {
                text: TextOptions::ui_text(Some(appearance.ui_font_size()), appearance),
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text("Enter a URL to preview", ctx);
            editor
        });

        ctx.subscribe_to_view(&url_editor, |me, _, event, ctx| {
            if matches!(event, EditorEvent::Enter) {
                me.submit_url_from_editor(ctx);
            }
        });

        let (frame_tick_tx, frame_tick_rx) = futures::channel::mpsc::unbounded::<()>();
        ctx.spawn_stream_local(
            frame_tick_rx,
            |me, _tick, ctx| {
                me.frame_sync_armed = false;
                me.sync_web_view(ctx);
                me.arm_frame_sync(ctx);
            },
            |_, _| {},
        );

        // Arm the initial post-frame sync.
        {
            let tx = frame_tick_tx.clone();
            ctx.on_next_frame_drawn(move || {
                let _ = tx.unbounded_send(());
            });
        }

        Self {
            url_editor,
            current_url: None,
            back_mouse_state: Default::default(),
            forward_mouse_state: Default::default(),
            reload_mouse_state: Default::default(),
            go_mouse_state: Default::default(),
            frame_tick_tx,
            frame_sync_armed: true,
        }
    }

    /// Navigates the preview to `url`, updating the URL bar to match. Accepts a
    /// bare host/path and prepends `https://` when no scheme is present.
    pub fn navigate(&mut self, url: impl Into<String>, ctx: &mut ViewContext<Self>) {
        let normalized = normalize_url(&url.into());
        if normalized.is_empty() {
            return;
        }

        self.url_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(&normalized, ctx);
        });
        self.current_url = Some(normalized.clone());

        if let Some(window) = ctx.windows().platform_window(ctx.window_id()) {
            let window = window.as_ref();
            window.ensure_web_view(BROWSER_PREVIEW_WEB_VIEW_ID);
            window.web_view_navigate(BROWSER_PREVIEW_WEB_VIEW_ID, &normalized);
        }
        ctx.notify();
    }

    fn submit_url_from_editor(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self.url_editor.as_ref(ctx).buffer_text(ctx);
        self.navigate(text, ctx);
    }

    /// Arms a single post-frame callback that re-syncs the native web view.
    fn arm_frame_sync(&mut self, ctx: &mut ViewContext<Self>) {
        if self.frame_sync_armed {
            return;
        }
        self.frame_sync_armed = true;
        let tx = self.frame_tick_tx.clone();
        ctx.on_next_frame_drawn(move || {
            let _ = tx.unbounded_send(());
        });
    }

    /// Positions and shows/hides the native web view to match the laid-out
    /// placeholder rect from the previous frame.
    fn sync_web_view(&self, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        let rect = ctx.element_position_by_id_at_last_frame(window_id, BROWSER_PREVIEW_WEB_VIEW_ID);
        let Some(window) = ctx.windows().platform_window(window_id) else {
            return;
        };
        let window = window.as_ref();

        match rect {
            Some(rect) => {
                window.ensure_web_view(BROWSER_PREVIEW_WEB_VIEW_ID);
                // Layout coordinates live in the zoomed-down layout space; scale
                // back up to real window points for the native view.
                let zoom = ctx.zoom_factor();
                // Inset horizontally so the native view doesn't cover the
                // panel's resize dragbar (see `WEB_VIEW_RESIZE_MARGIN`).
                let margin = WEB_VIEW_RESIZE_MARGIN;
                let inset_origin = vec2f(rect.origin().x() + margin, rect.origin().y());
                let inset_size = vec2f((rect.size().x() - margin * 2.0).max(0.0), rect.size().y());
                let scaled = RectF::new(inset_origin.scale_up(zoom), inset_size.scale_up(zoom));
                window.web_view_set_frame(BROWSER_PREVIEW_WEB_VIEW_ID, scaled);
                // Only reveal the web view once we have navigated somewhere;
                // otherwise the placeholder shows through.
                window.web_view_set_hidden(BROWSER_PREVIEW_WEB_VIEW_ID, self.current_url.is_none());
            }
            None => {
                // The preview tab isn't currently painted (drawer closed or a
                // different tab is active); hide the native view so it doesn't
                // float over unrelated UI.
                window.web_view_set_hidden(BROWSER_PREVIEW_WEB_VIEW_ID, true);
            }
        }
    }

    fn nav_button(
        &self,
        icon: Icon,
        mouse_state: MouseStateHandle,
        tooltip_text: &str,
        action: BrowserPreviewAction,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon_color = appearance
            .theme()
            .sub_text_color(appearance.theme().background())
            .into_solid();
        let tooltip = appearance
            .ui_builder()
            .tool_tip(tooltip_text.to_string())
            .build()
            .finish();

        icon_button(appearance, icon, false, mouse_state)
            .with_tooltip(move || tooltip)
            .with_style(UiComponentStyles {
                font_color: Some(icon_color),
                height: Some(24.),
                width: Some(24.),
                padding: Some(Coords::uniform(4.)),
                ..Default::default()
            })
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(action.clone());
            })
            .with_cursor(Cursor::PointingHand)
            .finish()
    }
}

impl Entity for BrowserPreviewView {
    type Event = ();
}

impl TypedActionView for BrowserPreviewView {
    type Action = BrowserPreviewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            BrowserPreviewAction::SubmitUrl => self.submit_url_from_editor(ctx),
            BrowserPreviewAction::Reload => {
                if let Some(window) = ctx.windows().platform_window(ctx.window_id()) {
                    window.as_ref().web_view_reload(BROWSER_PREVIEW_WEB_VIEW_ID);
                }
            }
            BrowserPreviewAction::Back => {
                if let Some(window) = ctx.windows().platform_window(ctx.window_id()) {
                    window
                        .as_ref()
                        .web_view_go_back(BROWSER_PREVIEW_WEB_VIEW_ID);
                }
            }
            BrowserPreviewAction::Forward => {
                if let Some(window) = ctx.windows().platform_window(ctx.window_id()) {
                    window
                        .as_ref()
                        .web_view_go_forward(BROWSER_PREVIEW_WEB_VIEW_ID);
                }
            }
        }
    }
}

impl View for BrowserPreviewView {
    fn ui_name() -> &'static str {
        "BrowserPreviewView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.url_editor);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        // Focus drives the border color so the URL field reads as an active
        // input; the background stays the app's default input grey.
        let url_focused = self.url_editor.is_focused(app);
        let border_fill = if url_focused {
            theme.accent()
        } else {
            theme.outline()
        };
        // Constrain the editor to a single line and pad it evenly so the text is
        // vertically centered within the input box (rather than stretching to
        // the box height and sitting at the top).
        let line_height = self
            .url_editor
            .as_ref(app)
            .line_height(app.font_cache(), appearance);
        let url_input = Container::new(
            ConstrainedBox::new(ChildView::new(&self.url_editor).finish())
                .with_height(line_height)
                .finish(),
        )
        .with_background(theme.surface_2())
        .with_border(Border::all(1.).with_border_fill(border_fill))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .with_padding_left(8.)
        .with_padding_right(8.)
        .with_padding_top(5.)
        .with_padding_bottom(5.)
        .finish();

        let nav_bar = ConstrainedBox::new(
            Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(4.0)
                    .with_child(self.nav_button(
                        Icon::ArrowLeft,
                        self.back_mouse_state.clone(),
                        "Back",
                        BrowserPreviewAction::Back,
                        appearance,
                    ))
                    .with_child(self.nav_button(
                        Icon::ArrowRight,
                        self.forward_mouse_state.clone(),
                        "Forward",
                        BrowserPreviewAction::Forward,
                        appearance,
                    ))
                    .with_child(self.nav_button(
                        Icon::Refresh,
                        self.reload_mouse_state.clone(),
                        "Reload",
                        BrowserPreviewAction::Reload,
                        appearance,
                    ))
                    .with_child(Expanded::new(1.0, url_input).finish())
                    .with_child(self.nav_button(
                        Icon::Globe,
                        self.go_mouse_state.clone(),
                        "Go",
                        BrowserPreviewAction::SubmitUrl,
                        appearance,
                    ))
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Start)
                    .finish(),
            )
            .with_padding_left(8.)
            .with_padding_right(8.)
            .finish(),
        )
        .with_height(NAV_BAR_HEIGHT as f32)
        .finish();

        // Content region reported to the geometry sync loop via `SavePosition`.
        // The native web view is composited above this rect, so we only paint a
        // placeholder here (centered) while the web view is hidden.
        let placeholder: Box<dyn Element> = if self.current_url.is_none() {
            Text::new(
                "Enter a URL above to preview a live page".to_string(),
                appearance.ui_font_family(),
                13.,
            )
            .with_color(theme.sub_text_color(theme.background()).into_solid())
            .finish()
        } else {
            Empty::new().finish()
        };

        let content_region = SavePosition::new(
            Align::new(placeholder).finish(),
            BROWSER_PREVIEW_WEB_VIEW_ID,
        )
        .for_single_frame()
        .finish();

        Flex::column()
            .with_child(nav_bar)
            .with_child(Expanded::new(1.0, content_region).finish())
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .finish()
    }
}

/// Adds a default `https://` scheme when the input lacks one, so bare hosts like
/// `localhost:3000` load correctly.
fn normalize_url(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    }
}
