use warpui::elements::{ClippedScrollStateHandle, ClippedScrollable, Element, Fill};
use warpui::{
    AppContext, Entity, ModelAsRef, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext,
};

use super::section_views::changelog_section::render_changelog_content;
use super::section_views::{SectionAction, SCROLLBAR_WIDTH};
use crate::appearance::Appearance;
use crate::changelog_model::{ChangelogModel, Event as ChangelogEvent};
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::TelemetryEvent;

/// Body of the "What's new" modal. Renders the latest changelog using the same
/// building blocks as the Resource Center changelog section, but without the
/// collapsible section chrome. Hosted by the workspace inside a generic
/// [`crate::modal::Modal`].
pub struct ChangelogModalBody {
    changelog_model_handle: ModelHandle<ChangelogModel>,
    scroll_state: ClippedScrollStateHandle,
}

impl ChangelogModalBody {
    pub fn new(
        changelog_model_handle: ModelHandle<ChangelogModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(
            &changelog_model_handle,
            |_me, _, _event: &ChangelogEvent, ctx| {
                ctx.notify();
            },
        );

        Self {
            changelog_model_handle,
            scroll_state: Default::default(),
        }
    }
}

impl Entity for ChangelogModalBody {
    type Event = ();
}

impl TypedActionView for ChangelogModalBody {
    type Action = SectionAction;

    fn handle_action(&mut self, action: &SectionAction, ctx: &mut ViewContext<Self>) {
        if let SectionAction::OpenUrl(url) = action {
            send_telemetry_from_ctx!(TelemetryEvent::OpenChangelogLink { url: url.clone() }, ctx);
            ctx.open_url(url.as_str());
        }
    }
}

impl View for ChangelogModalBody {
    fn ui_name() -> &'static str {
        "ChangelogModalBody"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let model = app.model(&self.changelog_model_handle);
        let appearance = Appearance::as_ref(app);
        let content = render_changelog_content(model, appearance);

        ClippedScrollable::vertical(
            self.scroll_state.clone(),
            content,
            SCROLLBAR_WIDTH,
            appearance
                .theme()
                .disabled_text_color(appearance.theme().background())
                .into(),
            appearance
                .theme()
                .main_text_color(appearance.theme().background())
                .into(),
            Fill::None,
        )
        .finish()
    }
}
