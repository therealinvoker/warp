//! Marketplace settings page.
//!
//! A directory browser for installable capabilities, backed by the Bang
//! backend's `SearchMarketplace` op. Three sources, one card list:
//!
//! - **Your org** — the workspace org-manifest entries (Marketplace M2 pipeline).
//! - **MCP registry** — the official public MCP server directory.
//! - **Plugins (Open VSX)** — Cursor-compatible extensions.
//!
//! "Get" installs into the selected drive space (personal or a team): MCP
//! entries become templatable MCP server cloud objects (plus a local
//! installation); plugin entries become marketplace-plugin drive objects.

use warpui::elements::{
    Border, Container, CornerRadius, CrossAxisAlignment, Element, Flex, MainAxisSize,
    MouseStateHandle, ParentElement, Radius, Shrinkable,
};
use warpui::presenter::ChildView;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use super::settings_page::{
    render_settings_info_banner, render_sub_header, MatchData, PageType, SettingsPageMeta,
    SettingsPageViewHandle, SettingsWidget,
};
use super::SettingsSection;
use crate::ai::mcp::parsing::ParsedTemplatableMCPServerResult;
use crate::ai::mcp::{ServerOrigin, TemplatableMCPServerManager};
use crate::appearance::Appearance;
use crate::cloud_object::{CloudObjectEventEntrypoint, Space};
use crate::editor::{Event as EditorEvent, EditorView, SingleLineEditorOptions};
use crate::marketplace_plugins::{CloudMarketplacePluginModel, MarketplacePlugin, PluginSource};
use crate::server::cloud_objects::update_manager::{InitiatedBy, UpdateManager};
use crate::server::ids::ClientId;
use crate::server::server_api::marketplace::{
    MarketplaceEntryKind, MarketplaceSearchEntry, MarketplaceSourceKind,
};
use crate::server::server_api::ServerApiProvider;
use crate::view_components::DismissibleToast;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::ToastStack;

const PAGE_TITLE_TEXT: &str = "Marketplace";
const BUTTON_FONT_SIZE: f32 = 12.;
const CARD_PADDING: f32 = 12.;

/// Loading / data / error state for the current directory query.
#[derive(Debug, Default)]
enum LoadState {
    #[default]
    Idle,
    Loading,
    Loaded(Vec<MarketplaceSearchEntry>),
    Error(String),
}

#[derive(Debug, Clone)]
pub enum MarketplacePageAction {
    SelectSource(MarketplaceSourceKind),
    SelectInstallSpace(Space),
    Search,
    /// Install the entry at this index of the loaded results.
    Install(usize),
}

pub enum MarketplacePageEvent {}

pub struct MarketplacePageView {
    page: PageType<Self>,
    source: MarketplaceSourceKind,
    load_state: LoadState,
    search_editor: ViewHandle<EditorView>,
    /// Which drive space "Get" installs into: personal or one of the teams.
    install_space: Space,
    space_options: Vec<(Space, String, MouseStateHandle)>,
    source_tabs: Vec<(MarketplaceSourceKind, &'static str, MouseStateHandle)>,
    search_button_state: MouseStateHandle,
}

impl Entity for MarketplacePageView {
    type Event = MarketplacePageEvent;
}

impl MarketplacePageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let search_editor = ctx.add_typed_action_view(|ctx| {
            let mut editor = EditorView::single_line(SingleLineEditorOptions::default(), ctx);
            editor.set_placeholder_text("Search MCP servers and plugins", ctx);
            editor
        });
        ctx.subscribe_to_view(&search_editor, |me, _, event, ctx| {
            if matches!(event, EditorEvent::Enter) {
                me.search(ctx);
            }
        });

        Self {
            page: PageType::new_monolith(MarketplaceWidget, Some(PAGE_TITLE_TEXT), false),
            source: MarketplaceSourceKind::Org,
            load_state: LoadState::Idle,
            search_editor,
            install_space: Space::Personal,
            space_options: Vec::new(),
            source_tabs: vec![
                (MarketplaceSourceKind::Org, "Your org", Default::default()),
                (
                    MarketplaceSourceKind::McpRegistry,
                    "MCP registry",
                    Default::default(),
                ),
                (
                    MarketplaceSourceKind::OpenVsx,
                    "Plugins (Open VSX)",
                    Default::default(),
                ),
            ],
            search_button_state: Default::default(),
        }
    }

    fn rebuild_space_options(&mut self, app: &AppContext) {
        let mut options = vec![(
            Space::Personal,
            "Personal".to_string(),
            MouseStateHandle::default(),
        )];
        for workspace in UserWorkspaces::as_ref(app).workspaces() {
            for team in &workspace.teams {
                options.push((
                    Space::Team { team_uid: team.uid },
                    team.name.clone(),
                    MouseStateHandle::default(),
                ));
            }
        }
        self.space_options = options;
    }

    fn query(&self, app: &AppContext) -> Option<String> {
        let text = self
            .search_editor
            .as_ref(app)
            .buffer_text(app)
            .trim()
            .to_owned();
        (!text.is_empty()).then_some(text)
    }

    fn search(&mut self, ctx: &mut ViewContext<Self>) {
        self.load_state = LoadState::Loading;
        ctx.notify();

        let client = ServerApiProvider::handle(ctx)
            .as_ref(ctx)
            .get_marketplace_client();
        let source = self.source;
        let query = self.query(ctx);
        ctx.spawn(
            async move { client.search_marketplace(source, query).await },
            |me, result, ctx| {
                me.load_state = match result {
                    Ok(entries) => LoadState::Loaded(entries),
                    Err(err) => LoadState::Error(err.to_string()),
                };
                ctx.notify();
            },
        );
    }

    /// Provenance for governance/telemetry: where an installed entry came from.
    fn origin_for_source(source: MarketplaceSourceKind) -> ServerOrigin {
        match source {
            MarketplaceSourceKind::Org => ServerOrigin::OrgMarketplace,
            MarketplaceSourceKind::McpRegistry | MarketplaceSourceKind::OpenVsx => {
                ServerOrigin::Registry
            }
        }
    }

    fn install(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        let LoadState::Loaded(entries) = &self.load_state else {
            return;
        };
        let Some(entry) = entries.get(index).cloned() else {
            return;
        };
        match entry.kind {
            MarketplaceEntryKind::Mcp => self.install_mcp(&entry, ctx),
            MarketplaceEntryKind::Plugin => self.install_plugin(&entry, ctx),
        }
    }

    fn install_mcp(&mut self, entry: &MarketplaceSearchEntry, ctx: &mut ViewContext<Self>) {
        let Some(template_json) = entry.mcp_template_json.as_deref() else {
            self.show_toast("This entry has no installable MCP config.".to_owned(), ctx);
            return;
        };
        let parsed_servers = match ParsedTemplatableMCPServerResult::from_user_json(template_json) {
            Ok(parsed) if !parsed.is_empty() => parsed,
            _ => {
                self.show_toast("Couldn't parse this entry's MCP config.".to_owned(), ctx);
                return;
            }
        };

        let space = self.install_space;
        let origin = Self::origin_for_source(self.source);
        for parsed_server in parsed_servers {
            let mut server = parsed_server.templatable_mcp_server.clone();
            server.origin = origin;
            TemplatableMCPServerManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.create_templatable_mcp_server(server, space, InitiatedBy::User, ctx);
                if let Some(installation) = parsed_server.templatable_mcp_server_installation {
                    manager.install_from_template(
                        installation.templatable_mcp_server().clone(),
                        installation.variable_values().clone(),
                        true,
                        ctx,
                    );
                }
            });
        }
        self.show_toast(format!("Added {} to your MCP servers.", entry.title), ctx);
    }

    fn install_plugin(&mut self, entry: &MarketplaceSearchEntry, ctx: &mut ViewContext<Self>) {
        let source = match (&entry.extension_id, &entry.bundle_url) {
            (Some(extension_id), _) => PluginSource::CursorExtension {
                extension_id: extension_id.clone(),
            },
            (None, Some(bundle_url)) => PluginSource::Url {
                bundle_url: bundle_url.clone(),
            },
            (None, None) => {
                self.show_toast("This entry has no installable source.".to_owned(), ctx);
                return;
            }
        };
        let plugin = MarketplacePlugin {
            uuid: uuid::Uuid::new_v4(),
            name: entry.title.clone(),
            description: (!entry.description.is_empty()).then(|| entry.description.clone()),
            source,
            pinned_version: entry.version.clone(),
            origin: Self::origin_for_source(self.source),
        };

        let Some(owner) =
            UserWorkspaces::as_ref(ctx).space_to_owner(self.install_space, ctx)
        else {
            self.show_toast("Couldn't resolve the install destination.".to_owned(), ctx);
            return;
        };
        let client_id = ClientId::default();
        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            update_manager.create_object(
                CloudMarketplacePluginModel::new(plugin),
                owner,
                client_id,
                CloudObjectEventEntrypoint::ManagementUI,
                true,
                None,
                InitiatedBy::User,
                ctx,
            );
        });
        self.show_toast(format!("Added {} to your drive.", entry.title), ctx);
    }

    fn show_toast(&self, message: String, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            toast_stack.add_ephemeral_toast(DismissibleToast::default(message), window_id, ctx);
        });
    }

    fn render_chip_row<T: Copy + PartialEq + 'static>(
        selected: T,
        options: impl Iterator<Item = (T, String, MouseStateHandle)>,
        to_action: impl Fn(T) -> MarketplacePageAction + Copy + 'static,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.);
        for (value, label, mouse_state) in options {
            let variant = if value == selected {
                ButtonVariant::Accent
            } else {
                ButtonVariant::Outlined
            };
            row.add_child(
                appearance
                    .ui_builder()
                    .button(variant, mouse_state)
                    .with_centered_text_label(label)
                    .with_style(UiComponentStyles {
                        font_size: Some(BUTTON_FONT_SIZE),
                        ..Default::default()
                    })
                    .build()
                    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(to_action(value)))
                    .finish(),
            );
        }
        row.finish()
    }

    fn render_entry_card(
        &self,
        index: usize,
        entry: &MarketplaceSearchEntry,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let kind_label = match entry.kind {
            MarketplaceEntryKind::Mcp => "MCP",
            MarketplaceEntryKind::Plugin => "Plugin",
        };
        let mut subtitle_parts = vec![kind_label.to_owned()];
        if let Some(publisher) = &entry.publisher {
            subtitle_parts.push(publisher.clone());
        }
        if let Some(version) = &entry.version {
            subtitle_parts.push(format!("v{version}"));
        }
        subtitle_parts.push(entry.source_label.clone());

        let title = appearance
            .ui_builder()
            .span(entry.title.clone())
            .with_style(UiComponentStyles {
                font_size: Some(14.),
                font_color: Some(theme.main_text_color(theme.surface_2()).into()),
                ..Default::default()
            })
            .build()
            .finish();
        let subtitle = appearance
            .ui_builder()
            .span(subtitle_parts.join(" · "))
            .with_style(UiComponentStyles {
                font_size: Some(11.),
                font_color: Some(theme.sub_text_color(theme.surface_2()).into()),
                ..Default::default()
            })
            .build()
            .finish();

        let mut text_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(title)
            .with_child(Container::new(subtitle).with_margin_top(2.).finish());
        if !entry.description.is_empty() {
            let description: String = entry.description.chars().take(180).collect();
            text_column.add_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .paragraph(description)
                        .with_style(UiComponentStyles {
                            font_size: Some(12.),
                            font_color: Some(theme.sub_text_color(theme.surface_2()).into()),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_margin_top(4.)
                .finish(),
            );
        }

        let get_button = appearance
            .ui_builder()
            .button(ButtonVariant::Accent, MouseStateHandle::default())
            .with_centered_text_label("Get".to_owned())
            .with_style(UiComponentStyles {
                font_size: Some(BUTTON_FONT_SIZE),
                ..Default::default()
            })
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(MarketplacePageAction::Install(index))
            })
            .finish();

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Shrinkable::new(1., text_column.finish()).finish())
                .with_child(get_button)
                .finish(),
        )
        .with_uniform_padding(CARD_PADDING)
        .with_margin_bottom(8.)
        .with_background(theme.surface_2())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .with_border(Border::all(1.).with_border_fill(theme.outline()))
        .finish()
    }
}

impl View for MarketplacePageView {
    fn ui_name() -> &'static str {
        "MarketplacePageView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl TypedActionView for MarketplacePageView {
    type Action = MarketplacePageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            MarketplacePageAction::SelectSource(source) => {
                self.source = *source;
                self.search(ctx);
            }
            MarketplacePageAction::SelectInstallSpace(space) => {
                self.install_space = *space;
                ctx.notify();
            }
            MarketplacePageAction::Search => self.search(ctx),
            MarketplacePageAction::Install(index) => self.install(*index, ctx),
        }
    }
}

impl SettingsPageMeta for MarketplacePageView {
    fn section() -> SettingsSection {
        SettingsSection::Marketplace
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        true
    }

    fn on_page_selected(&mut self, _allow_steal_focus: bool, ctx: &mut ViewContext<Self>) {
        self.rebuild_space_options(ctx);
        if matches!(self.load_state, LoadState::Idle) {
            self.search(ctx);
        }
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

impl From<ViewHandle<MarketplacePageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<MarketplacePageView>) -> Self {
        SettingsPageViewHandle::Marketplace(view_handle)
    }
}

struct MarketplaceWidget;

impl SettingsWidget for MarketplaceWidget {
    type View = MarketplacePageView;

    fn search_terms(&self) -> &str {
        "marketplace mcp servers plugins extensions registry open vsx directory install"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Max);

        // Search input + button.
        let search_input =
            Container::new(ChildView::new(&view.search_editor).finish())
                .with_padding_top(8.)
                .with_padding_bottom(8.)
                .with_padding_left(12.)
                .with_padding_right(12.)
                .with_background(theme.background())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .finish();
        let search_button = appearance
            .ui_builder()
            .button(ButtonVariant::Outlined, view.search_button_state.clone())
            .with_centered_text_label("Search".to_owned())
            .with_style(UiComponentStyles {
                font_size: Some(BUTTON_FONT_SIZE),
                ..Default::default()
            })
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(MarketplacePageAction::Search))
            .finish();
        column.add_child(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(8.)
                .with_child(Shrinkable::new(1., search_input).finish())
                .with_child(search_button)
                .finish(),
        );

        // Source tabs.
        column.add_child(
            Container::new(MarketplacePageView::render_chip_row(
                view.source,
                view.source_tabs
                    .iter()
                    .map(|(source, label, state)| (*source, (*label).to_owned(), state.clone())),
                MarketplacePageAction::SelectSource,
                appearance,
            ))
            .with_margin_top(12.)
            .finish(),
        );

        // Install-target space picker.
        column.add_child(
            Container::new(render_sub_header(appearance, "Install to", None)).finish(),
        );
        column.add_child(MarketplacePageView::render_chip_row(
            view.install_space,
            view.space_options
                .iter()
                .map(|(space, name, state)| (*space, name.clone(), state.clone())),
            MarketplacePageAction::SelectInstallSpace,
            appearance,
        ));

        // Results.
        column.add_child(
            Container::new(render_sub_header(appearance, "Results", None)).finish(),
        );
        match &view.load_state {
            LoadState::Idle | LoadState::Loading => {
                column.add_child(
                    appearance
                        .ui_builder()
                        .span("Searching the directory…".to_owned())
                        .with_style(UiComponentStyles {
                            font_color: Some(theme.sub_text_color(theme.surface_2()).into()),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                );
            }
            LoadState::Error(message) => {
                column.add_child(render_settings_info_banner(message, None, appearance));
            }
            LoadState::Loaded(entries) if entries.is_empty() => {
                column.add_child(
                    appearance
                        .ui_builder()
                        .span("No results. Try a different search or source.".to_owned())
                        .with_style(UiComponentStyles {
                            font_color: Some(theme.sub_text_color(theme.surface_2()).into()),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                );
            }
            LoadState::Loaded(entries) => {
                for (index, entry) in entries.iter().enumerate() {
                    column.add_child(view.render_entry_card(index, entry, appearance));
                }
            }
        }

        column.finish()
    }
}
