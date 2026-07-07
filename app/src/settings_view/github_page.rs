//! GitHub integration settings page.
//!
//! Shows the user's GitHub connection status (username + installed repos) and
//! exposes connect / manage-installation / disconnect actions. First connect
//! goes through the backend's install-start hop (GitHub App install with
//! OAuth-during-installation: authorize + pick repos in one visit); reconnect
//! and stateless fallbacks use the server-mediated OAuth round-trip. Both
//! carry the `next=` deep link, here targeting
//! [`GithubAuthRedirectTarget::SettingsGithub`].
//!
//! Connection state lives on the [`GithubConnection`] singleton; this page
//! observes it and re-renders on [`GithubConnectionEvent::StateChanged`]. The
//! connect state machine mirrors `update_environment_form.rs` (refresh on
//! `GitHubAuthEvent::AuthCompleted`, open auth URL with `next=`).
//!
//! Gated on [`FeatureFlag::GithubIntegration`].

use pathfinder_geometry::vector::Vector2F;
use warp_core::features::FeatureFlag;
use warpui::elements::{
    Container, CrossAxisAlignment, Element, Flex, MainAxisSize, MouseStateHandle, ParentElement,
    Text,
};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Entity, EventContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use super::settings_page::{
    render_separator, render_sub_header, MatchData, PageType, SettingsPageMeta,
    SettingsPageViewHandle, SettingsWidget,
};
use super::SettingsSection;
use crate::ai::ambient_agents::github_auth_url::settings_github_auth_url_with_next;
use crate::appearance::Appearance;
use crate::github::{GithubConnection, GithubConnectionEvent};

const PAGE_TITLE_TEXT: &str = "GitHub";
const BUTTON_FONT_SIZE: f32 = 12.;

/// Actions dispatched by the GitHub settings page.
#[derive(Debug, Clone)]
pub enum GithubSettingsPageAction {
    /// Start (or restart) the GitHub connect flow in the browser.
    Connect,
    /// Open the GitHub App installation-management page.
    ManageInstallation,
    /// Re-fetch connection status from the backend.
    Refresh,
}

/// Events emitted by the GitHub settings page.
#[derive(Debug, Clone)]
pub enum GithubSettingsPageEvent {}

pub struct GithubSettingsPageView {
    page: PageType<Self>,
    connect_mouse_state: MouseStateHandle,
    manage_mouse_state: MouseStateHandle,
    refresh_mouse_state: MouseStateHandle,
}

impl Entity for GithubSettingsPageView {
    type Event = GithubSettingsPageEvent;
}

impl GithubSettingsPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        // Re-render whenever the connection state changes.
        ctx.subscribe_to_model(
            &GithubConnection::handle(ctx),
            |_, _, event, ctx| match event {
                GithubConnectionEvent::StateChanged => ctx.notify(),
            },
        );

        Self {
            page: PageType::new_monolith(GithubSettingsWidget, Some(PAGE_TITLE_TEXT), true),
            connect_mouse_state: MouseStateHandle::default(),
            manage_mouse_state: MouseStateHandle::default(),
            refresh_mouse_state: MouseStateHandle::default(),
        }
    }

    /// Open the connect flow, preferring the backend's install-start hop: it
    /// redirects to the GitHub App install page with OAuth-during-installation,
    /// so one GitHub visit both authorizes and picks repos. The hop covers
    /// reconnects too — repo-selection updates also return to the combined
    /// callback with a fresh code. A bare github.com install link (served when
    /// the user has no admin workspace, or the App isn't configured) carries no
    /// signed state, so the combined callback couldn't bind the user; those
    /// fall back to the plain OAuth auth URL. The base is wrapped with a
    /// `next=` deep link back to this page — the backend folds it into the
    /// signed state.
    fn connect(&mut self, ctx: &mut ViewContext<Self>) {
        let state = GithubConnection::as_ref(ctx).state().clone();
        let install_hop = state
            .app_install_link
            .clone()
            .filter(|link| link.contains("/github/install/start"));
        let Some(base) = install_hop.or(state.auth_url.clone()) else {
            // Nothing state-bound to open — the connection info is missing or
            // stale. The historical `{server}/oauth/connect/github` fallback
            // needs a browser session this backend doesn't have (it 404s), so
            // re-fetch instead; the page shows any load error and the next
            // click gets a fresh URL.
            self.refresh(ctx);
            return;
        };
        let url = settings_github_auth_url_with_next(&base);
        ctx.open_url(&url);
    }

    fn manage_installation(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(link) = GithubConnection::as_ref(ctx)
            .state()
            .app_install_link
            .clone()
        {
            // Wrap with the `next=` deep link so the backend routes the
            // browser back to this page after GitHub returns (the bare
            // github.com fallback link ignores the param; harmless there).
            let url = settings_github_auth_url_with_next(&link);
            ctx.open_url(&url);
        }
    }

    fn refresh(&mut self, ctx: &mut ViewContext<Self>) {
        GithubConnection::handle(ctx).update(ctx, |connection, ctx| connection.refresh(ctx));
    }

    fn render_button<F>(
        &self,
        text: &str,
        variant: ButtonVariant,
        mouse_state: MouseStateHandle,
        on_click: F,
        appearance: &Appearance,
    ) -> Box<dyn Element>
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F),
    {
        Container::new(
            appearance
                .ui_builder()
                .button(variant, mouse_state)
                .with_centered_text_label(text.to_owned())
                .with_style(UiComponentStyles {
                    font_size: Some(BUTTON_FONT_SIZE),
                    ..Default::default()
                })
                .build()
                .on_click(on_click)
                .finish(),
        )
        .with_margin_right(8.)
        .finish()
    }

    fn render_status_text(&self, text: String, appearance: &Appearance) -> Box<dyn Element> {
        Text::new_inline(text, appearance.ui_font_family(), appearance.ui_font_size())
            .with_color(appearance.theme().foreground().into())
            .finish()
    }
}

impl View for GithubSettingsPageView {
    fn ui_name() -> &'static str {
        "GithubSettingsPageView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl TypedActionView for GithubSettingsPageView {
    type Action = GithubSettingsPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            GithubSettingsPageAction::Connect => self.connect(ctx),
            GithubSettingsPageAction::ManageInstallation => self.manage_installation(ctx),
            GithubSettingsPageAction::Refresh => self.refresh(ctx),
        }
    }
}

impl SettingsPageMeta for GithubSettingsPageView {
    fn section() -> SettingsSection {
        SettingsSection::Github
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        FeatureFlag::GithubIntegration.is_enabled()
    }

    fn on_page_selected(&mut self, _allow_steal_focus: bool, ctx: &mut ViewContext<Self>) {
        // Refresh status whenever the page is opened.
        self.refresh(ctx);
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget()
    }
}

impl From<ViewHandle<GithubSettingsPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<GithubSettingsPageView>) -> Self {
        SettingsPageViewHandle::Github(view_handle)
    }
}

struct GithubSettingsWidget;

impl SettingsWidget for GithubSettingsWidget {
    type View = GithubSettingsPageView;

    fn search_terms(&self) -> &str {
        "github connect installation repositories pull request"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let state = GithubConnection::as_ref(app).state().clone();

        // Content-sized: this monolith page is dual-scrollable
        // (new_monolith(..., true)), so its content lays out under an unbounded
        // vertical constraint inside the scrollable. A Max main-axis flex panics
        // there; size to content instead and let the scrollable handle overflow.
        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Min);

        column.add_child(render_sub_header(appearance, "Connection", None));

        let status_line = if state.is_loading {
            "Checking GitHub connection…".to_string()
        } else if state.connected {
            match &state.username {
                Some(username) => format!("Connected as {username}"),
                None => "Connected".to_string(),
            }
        } else {
            "Not connected".to_string()
        };
        column.add_child(view.render_status_text(status_line, appearance));

        if let Some(error) = &state.load_error {
            column.add_child(
                Container::new(view.render_status_text(error.clone(), appearance))
                    .with_margin_top(4.)
                    .finish(),
            );
        }

        // Repos the user can see through the App (when connected). The list
        // is the user's own GitHub visibility; "automations" marks repos in a
        // workspace-claimed installation, which ambient agents may act on.
        if state.connected && !state.installed_repos.is_empty() {
            let total = state.installed_repos.len();
            let enabled = state
                .installed_repos
                .iter()
                .filter(|r| r.automation_enabled)
                .count();
            let summary = format!(
                "{} accessible {} · {} enabled for automations",
                total,
                if total == 1 {
                    "repository"
                } else {
                    "repositories"
                },
                enabled,
            );
            column.add_child(
                Container::new(view.render_status_text(summary, appearance))
                    .with_margin_top(8.)
                    .finish(),
            );
            for repo in state.installed_repos.iter() {
                let label = if repo.automation_enabled {
                    format!("{} · automations", repo.full_name())
                } else {
                    repo.full_name()
                };
                column.add_child(
                    Container::new(view.render_status_text(label, appearance))
                        .with_margin_top(2.)
                        .with_margin_left(8.)
                        .finish(),
                );
            }
        }

        column.add_child(
            Container::new(render_separator(appearance))
                .with_margin_top(16.)
                .with_margin_bottom(16.)
                .finish(),
        );

        // Action buttons.
        let mut buttons = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min);

        if state.connected {
            buttons.add_child(view.render_button(
                "Manage installation",
                ButtonVariant::Accent,
                view.manage_mouse_state.clone(),
                |ctx, _, _| ctx.dispatch_typed_action(GithubSettingsPageAction::ManageInstallation),
                appearance,
            ));
            // Disconnect is performed by revoking access from GitHub's
            // installation-management page (opened via "Manage installation").
            // A dedicated in-app disconnect mutation lands with the broader
            // installation-management ops; here we relabel the reconnect path.
            buttons.add_child(view.render_button(
                "Reconnect",
                ButtonVariant::Text,
                view.connect_mouse_state.clone(),
                |ctx, _, _| ctx.dispatch_typed_action(GithubSettingsPageAction::Connect),
                appearance,
            ));
        } else {
            buttons.add_child(view.render_button(
                "Connect GitHub",
                ButtonVariant::Accent,
                view.connect_mouse_state.clone(),
                |ctx, _, _| ctx.dispatch_typed_action(GithubSettingsPageAction::Connect),
                appearance,
            ));
        }

        buttons.add_child(view.render_button(
            "Refresh",
            ButtonVariant::Text,
            view.refresh_mouse_state.clone(),
            |ctx, _, _| ctx.dispatch_typed_action(GithubSettingsPageAction::Refresh),
            appearance,
        ));

        column.add_child(buttons.finish());

        column.finish()
    }
}
