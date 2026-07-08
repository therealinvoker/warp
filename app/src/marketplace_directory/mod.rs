//! The Bang Marketplace: a browsable directory pane hosted in the main
//! canvas (like Settings), modeled after Cursor's Marketplace — a left rail
//! (search + sections + install target) and a searchable grid of cards.
//!
//! Data is live, served by the Bang backend's `SearchMarketplace` op across
//! three directories:
//!
//! - **Your org** — workspace org-manifest entries (Marketplace M2 pipeline).
//! - **MCP servers** — the official public MCP registry.
//! - **Plugins** — Cursor-compatible extensions from Open VSX.
//!
//! The rail search queries ALL sources at once; sections filter the merged
//! results. "Get" installs into the selected drive space: MCP entries become
//! templatable MCP server cloud objects (installed locally too); plugin
//! entries become `MarketplacePlugin` drive objects.

pub mod pane_manager;

use warpui::elements::{
    Border, ChildView, Clipped, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    Expanded, Flex, Hoverable, MainAxisSize, MouseStateHandle, Padding, ParentElement, Radius,
    Shrinkable, Text, Wrap,
};
use warpui::fonts::Weight;
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, EventContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use crate::ai::mcp::parsing::ParsedTemplatableMCPServerResult;
use crate::ai::mcp::{ServerOrigin, TemplatableMCPServerManager};
use crate::appearance::Appearance;
use crate::cloud_object::{CloudObjectEventEntrypoint, Space};
use crate::editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions};
use crate::marketplace_plugins::{CloudMarketplacePluginModel, MarketplacePlugin, PluginSource};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view::{self, HeaderContent, StandardHeader, StandardHeaderOptions};
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent};
use crate::server::cloud_objects::update_manager::{InitiatedBy, UpdateManager};
use crate::server::ids::ClientId;
use crate::server::server_api::marketplace::{
    MarketplaceEntryKind, MarketplaceSearchEntry, MarketplaceSourceKind,
};
use crate::server::server_api::ServerApiProvider;
use crate::ui_components::avatar::{Avatar, AvatarContent};
use crate::ui_components::blended_colors;
use crate::view_components::DismissibleToast;
use crate::workspace::WorkspaceAction;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::ToastStack;

const NAV_WIDTH: f32 = 216.;
const CARD_WIDTH: f32 = 320.;
const CARD_SPACING: f32 = 12.;
const NAV_FONT_SIZE: f32 = 13.;
const CARD_TITLE_FONT_SIZE: f32 = 14.;
const CARD_BODY_FONT_SIZE: f32 = 12.;

const DOCUMENTATION_URL: &str = "https://modelcontextprotocol.io/docs";

/// Header text for the marketplace pane.
pub const MARKETPLACE_HEADER_TEXT: &str = "Marketplace";

/// The left-rail sections. `All` merges every directory; the rest filter to
/// one backing source.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    All,
    Org,
    McpRegistry,
    OpenVsx,
}

impl Section {
    const ALL: &'static [Section] = &[
        Section::All,
        Section::Org,
        Section::McpRegistry,
        Section::OpenVsx,
    ];

    fn label(self) -> &'static str {
        match self {
            Section::All => "All",
            Section::Org => "Your org",
            Section::McpRegistry => "MCP servers",
            Section::OpenVsx => "Plugins",
        }
    }
}

/// The sources fetched by the rail search, in merge/display order.
const SOURCES: [MarketplaceSourceKind; 3] = [
    MarketplaceSourceKind::Org,
    MarketplaceSourceKind::McpRegistry,
    MarketplaceSourceKind::OpenVsx,
];

fn source_index(source: MarketplaceSourceKind) -> usize {
    SOURCES
        .iter()
        .position(|s| *s == source)
        .expect("source is one of SOURCES")
}

/// Per-directory fetch state.
#[derive(Debug, Default)]
enum SourceState {
    #[default]
    Idle,
    Loading,
    Loaded(Vec<MarketplaceSearchEntry>),
    Error(String),
}

#[derive(Debug, Clone)]
pub enum MarketplaceDirectoryAction {
    SelectSection(usize),
    SelectInstallSpace(usize),
    /// Install the entry with this id from this source.
    Install(MarketplaceSourceKind, String),
    OpenCustomize,
    OpenDocumentation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketplaceDirectoryEvent {
    Pane(PaneEvent),
}

/// Overflow-menu actions for the pane header (currently none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketplaceHeaderAction {}

/// Custom header actions (currently none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketplaceHeaderCustomAction {}

pub struct MarketplaceDirectoryView {
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    selected_section: Section,
    /// Which drive space "Get" installs into.
    install_space: Space,
    space_options: Vec<(Space, String, MouseStateHandle)>,
    search_editor: ViewHandle<EditorView>,
    /// Fetch state per entry of [`SOURCES`].
    source_states: [SourceState; 3],
    /// The query the current `source_states` were fetched with; a stale-guard
    /// so an old in-flight response can't clobber a newer search.
    fetched_query: String,
    customize_mouse_state: MouseStateHandle,
    documentation_mouse_state: MouseStateHandle,
    section_mouse_states: Vec<MouseStateHandle>,
    card_mouse_states: Vec<MouseStateHandle>,
}

impl MarketplaceDirectoryView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let search_editor = ctx.add_typed_action_view(|ctx| {
            let mut editor = EditorView::single_line(SingleLineEditorOptions::default(), ctx);
            editor.set_placeholder_text("Search MCPs and plugins", ctx);
            editor
        });
        // Filter the fetched results live as the user types; Enter re-queries
        // every directory server-side (registry/Open VSX search beats
        // client-side substring filtering of the default listing).
        ctx.subscribe_to_view(&search_editor, |me, _, event, ctx| match event {
            EditorEvent::Enter => me.refresh(ctx),
            _ => ctx.notify(),
        });

        let pane_configuration =
            ctx.add_model(|_ctx| PaneConfiguration::new(MARKETPLACE_HEADER_TEXT));

        let mut view = Self {
            pane_configuration,
            focus_handle: None,
            selected_section: Section::All,
            install_space: Space::Personal,
            space_options: Vec::new(),
            search_editor,
            source_states: Default::default(),
            fetched_query: String::new(),
            customize_mouse_state: Default::default(),
            documentation_mouse_state: Default::default(),
            section_mouse_states: Section::ALL
                .iter()
                .map(|_| MouseStateHandle::default())
                .collect(),
            card_mouse_states: Vec::new(),
        };
        view.rebuild_space_options(ctx);
        view.refresh(ctx);
        view
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    pub fn focus(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.search_editor);
    }

    /// Re-syncs teams and re-queries every directory. Called when the user
    /// re-triggers the open action while the pane is already open.
    pub fn reopen(&mut self, ctx: &mut ViewContext<Self>) {
        self.rebuild_space_options(ctx);
        self.refresh(ctx);
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
        if !self
            .space_options
            .iter()
            .any(|(space, _, _)| *space == self.install_space)
        {
            self.install_space = Space::Personal;
        }
    }

    fn search_term(&self, app: &AppContext) -> String {
        self.search_editor
            .as_ref(app)
            .buffer_text(app)
            .trim()
            .to_string()
    }

    /// Fires one search per directory with the current query. Each response
    /// lands independently; a stale-guard drops responses from older queries.
    fn refresh(&mut self, ctx: &mut ViewContext<Self>) {
        let term = self.search_term(ctx);
        self.fetched_query = term.clone();
        let query = (!term.is_empty()).then_some(term.clone());

        for source in SOURCES {
            self.source_states[source_index(source)] = SourceState::Loading;
            let client = ServerApiProvider::handle(ctx)
                .as_ref(ctx)
                .get_marketplace_client();
            let query = query.clone();
            let term = term.clone();
            ctx.spawn(
                async move {
                    let result = client.search_marketplace(source, query).await;
                    (source, term, result)
                },
                |me, (source, term, result), ctx| {
                    if term != me.fetched_query {
                        return; // superseded by a newer search
                    }
                    me.source_states[source_index(source)] = match result {
                        Ok(entries) => SourceState::Loaded(entries),
                        Err(err) => SourceState::Error(err.to_string()),
                    };
                    me.resize_card_mouse_states();
                    ctx.notify();
                },
            );
        }
        ctx.notify();
    }

    fn resize_card_mouse_states(&mut self) {
        let total: usize = self
            .source_states
            .iter()
            .map(|state| match state {
                SourceState::Loaded(entries) => entries.len(),
                _ => 0,
            })
            .sum();
        while self.card_mouse_states.len() < total {
            self.card_mouse_states.push(MouseStateHandle::default());
        }
    }

    /// The entries visible under the current section + live text filter, as
    /// `(source, entry)` pairs in merged display order.
    fn visible_entries(
        &self,
        app: &AppContext,
    ) -> Vec<(MarketplaceSourceKind, &MarketplaceSearchEntry)> {
        let term = self.search_term(app).to_lowercase();
        let wanted_source = match self.selected_section {
            Section::All => None,
            Section::Org => Some(MarketplaceSourceKind::Org),
            Section::McpRegistry => Some(MarketplaceSourceKind::McpRegistry),
            Section::OpenVsx => Some(MarketplaceSourceKind::OpenVsx),
        };

        let mut visible = Vec::new();
        for source in SOURCES {
            if wanted_source.is_some_and(|wanted| wanted != source) {
                continue;
            }
            if let SourceState::Loaded(entries) = &self.source_states[source_index(source)] {
                for entry in entries {
                    let matches = term.is_empty()
                        || entry.title.to_lowercase().contains(&term)
                        || entry.description.to_lowercase().contains(&term)
                        || entry
                            .publisher
                            .as_deref()
                            .is_some_and(|p| p.to_lowercase().contains(&term));
                    if matches {
                        visible.push((source, entry));
                    }
                }
            }
        }
        visible
    }

    /// True while any directory fetch for the current query is in flight.
    fn any_loading(&self) -> bool {
        self.source_states
            .iter()
            .any(|state| matches!(state, SourceState::Loading))
    }

    /// Error messages from directories that failed for the current query.
    fn errors(&self) -> Vec<&str> {
        self.source_states
            .iter()
            .filter_map(|state| match state {
                SourceState::Error(message) => Some(message.as_str()),
                _ => None,
            })
            .collect()
    }

    // ── Install ────────────────────────────────────────────────────────────

    fn install(
        &mut self,
        source: MarketplaceSourceKind,
        entry_id: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        let SourceState::Loaded(entries) = &self.source_states[source_index(source)] else {
            return;
        };
        let Some(entry) = entries
            .iter()
            .find(|entry| entry.entry_id.inner() == entry_id)
            .cloned()
        else {
            return;
        };
        match entry.kind {
            MarketplaceEntryKind::Mcp => self.install_mcp(source, &entry, ctx),
            MarketplaceEntryKind::Plugin => self.install_plugin(source, &entry, ctx),
        }
    }

    fn origin_for_source(source: MarketplaceSourceKind) -> ServerOrigin {
        match source {
            MarketplaceSourceKind::Org => ServerOrigin::OrgMarketplace,
            MarketplaceSourceKind::McpRegistry | MarketplaceSourceKind::OpenVsx => {
                ServerOrigin::Registry
            }
        }
    }

    fn install_mcp(
        &mut self,
        source: MarketplaceSourceKind,
        entry: &MarketplaceSearchEntry,
        ctx: &mut ViewContext<Self>,
    ) {
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
        let origin = Self::origin_for_source(source);
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

    fn install_plugin(
        &mut self,
        source: MarketplaceSourceKind,
        entry: &MarketplaceSearchEntry,
        ctx: &mut ViewContext<Self>,
    ) {
        let plugin_source = match (&entry.extension_id, &entry.bundle_url) {
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
            source: plugin_source,
            pinned_version: entry.version.clone(),
            origin: Self::origin_for_source(source),
        };

        let Some(owner) = UserWorkspaces::as_ref(ctx).space_to_owner(self.install_space, ctx)
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

    // ── Rendering ──────────────────────────────────────────────────────────

    fn render_nav_item(
        label: &str,
        selected: bool,
        mouse_state: MouseStateHandle,
        appearance: &Appearance,
        on_click: impl Fn(&mut EventContext) + 'static,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text_color = if selected {
            blended_colors::text_main(theme, theme.surface_3())
        } else {
            blended_colors::text_sub(theme, theme.surface_2())
        };

        let label = label.to_string();
        let font_family = appearance.ui_font_family();
        let surface_2 = theme.surface_2();
        let surface_3 = theme.surface_3();

        Hoverable::new(mouse_state, move |state| {
            let row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(8.)
                .with_child(
                    Text::new(label.clone(), font_family, NAV_FONT_SIZE)
                        .with_color(text_color)
                        .with_selectable(false)
                        .finish(),
                );

            let mut container = Container::new(row.finish())
                .with_padding_left(10.)
                .with_padding_right(10.)
                .with_padding_top(6.)
                .with_padding_bottom(6.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)));
            if selected {
                container = container.with_background(surface_3);
            } else if state.is_hovered() {
                container = container.with_background(surface_2);
            }
            container.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| on_click(ctx))
        .finish()
    }

    fn render_nav_heading(text: &str, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        Container::new(
            Text::new(text.to_string(), appearance.ui_font_family(), 11.)
                .with_color(blended_colors::text_disabled(theme, theme.surface_2()))
                .with_selectable(false)
                .finish(),
        )
        .with_padding_left(10.)
        .with_padding_top(12.)
        .with_padding_bottom(4.)
        .finish()
    }

    fn render_nav(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut column = Flex::column().with_main_axis_size(MainAxisSize::Max);

        // Search across every source, right in the rail.
        column = column.with_child(
            Container::new(
                Container::new(Clipped::new(ChildView::new(&self.search_editor).finish()).finish())
                    .with_background(theme.background())
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                    .with_border(Border::all(1.).with_border_fill(theme.outline()))
                    .with_padding_left(10.)
                    .with_padding_right(10.)
                    .with_padding_top(7.)
                    .with_padding_bottom(7.)
                    .finish(),
            )
            .with_padding_left(2.)
            .with_padding_right(2.)
            .with_margin_bottom(4.)
            .finish(),
        );

        column = column.with_child(Self::render_nav_heading("BROWSE", appearance));
        for (index, section) in Section::ALL.iter().enumerate() {
            let selected = *section == self.selected_section;
            column = column.with_child(Self::render_nav_item(
                section.label(),
                selected,
                self.section_mouse_states[index].clone(),
                appearance,
                move |ctx| {
                    ctx.dispatch_typed_action(MarketplaceDirectoryAction::SelectSection(index))
                },
            ));
        }

        column = column.with_child(Self::render_nav_heading("INSTALL TO", appearance));
        for (index, (space, name, mouse_state)) in self.space_options.iter().enumerate() {
            let selected = *space == self.install_space;
            column = column.with_child(Self::render_nav_item(
                name,
                selected,
                mouse_state.clone(),
                appearance,
                move |ctx| {
                    ctx.dispatch_typed_action(MarketplaceDirectoryAction::SelectInstallSpace(index))
                },
            ));
        }

        column = column.with_child(Self::render_nav_heading("RESOURCES", appearance));
        column = column.with_child(Self::render_nav_item(
            "Customize",
            false,
            self.customize_mouse_state.clone(),
            appearance,
            |ctx| ctx.dispatch_typed_action(MarketplaceDirectoryAction::OpenCustomize),
        ));
        column = column.with_child(Self::render_nav_item(
            "Documentation",
            false,
            self.documentation_mouse_state.clone(),
            appearance,
            |ctx| ctx.dispatch_typed_action(MarketplaceDirectoryAction::OpenDocumentation),
        ));

        Container::new(Clipped::new(column.finish()).finish())
            .with_padding_left(8.)
            .with_padding_right(8.)
            .with_padding_top(12.)
            .with_border(Border::right(1.).with_border_fill(theme.outline()))
            .finish()
    }

    fn render_card(
        source: MarketplaceSourceKind,
        entry: &MarketplaceSearchEntry,
        mouse_state: MouseStateHandle,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let title_text = entry.title.clone();
        let kind_label = match entry.kind {
            MarketplaceEntryKind::Mcp => "MCP",
            MarketplaceEntryKind::Plugin => "Plugin",
        };
        let mut subtitle_parts = vec![kind_label.to_owned()];
        if let Some(publisher) = &entry.publisher {
            subtitle_parts.push(publisher.clone());
        }
        subtitle_parts.push(entry.source_label.clone());
        let subtitle_text = subtitle_parts.join(" · ");
        let description_text: String = entry.description.chars().take(140).collect();
        let entry_id = entry.entry_id.inner().to_owned();
        let font_family = appearance.ui_font_family();
        let icon_url = entry.icon_url.clone();

        Hoverable::new(mouse_state, move |state| {
            // Real directory icon when the entry has one (Open VSX always
            // does); the display-name initial otherwise, which is also the
            // Image variant's before-load placeholder.
            let avatar_content = match &icon_url {
                Some(url) => AvatarContent::Image {
                    url: url.clone(),
                    display_name: title_text.clone(),
                },
                None => AvatarContent::DisplayName(title_text.clone()),
            };
            let avatar = Avatar::new(
                avatar_content,
                UiComponentStyles {
                    width: Some(32.),
                    height: Some(32.),
                    border_radius: Some(CornerRadius::with_all(Radius::Pixels(6.))),
                    font_family_id: Some(font_family),
                    font_weight: Some(Weight::Bold),
                    background: Some(theme.background().into()),
                    font_size: Some(16.),
                    font_color: Some(blended_colors::text_main(theme, theme.background())),
                    ..Default::default()
                },
            )
            .build()
            .finish();

            let name = Text::new(title_text.clone(), font_family, CARD_TITLE_FONT_SIZE)
                .with_color(blended_colors::text_main(theme, theme.surface_1()))
                .with_selectable(false)
                .finish();

            let subtitle = Text::new(subtitle_text.clone(), font_family, 11.)
                .with_color(blended_colors::text_disabled(theme, theme.surface_1()))
                .with_selectable(false)
                .finish();

            let description = Text::new(description_text.clone(), font_family, CARD_BODY_FONT_SIZE)
                .with_color(blended_colors::text_sub(theme, theme.surface_1()))
                .with_selectable(false)
                .finish();

            let info_column = Flex::column()
                .with_child(name)
                .with_child(Container::new(subtitle).with_margin_top(2.).finish())
                .with_child(Container::new(description).with_margin_top(2.).finish())
                .finish();

            let cta = Container::new(
                Text::new("Get".to_string(), font_family, CARD_BODY_FONT_SIZE)
                    .with_color(blended_colors::text_main(theme, theme.surface_3()))
                    .with_selectable(false)
                    .finish(),
            )
            .with_background(theme.surface_3())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .with_padding_left(12.)
            .with_padding_right(12.)
            .with_padding_top(5.)
            .with_padding_bottom(5.)
            .finish();

            let mut card = Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(10.)
                    .with_child(avatar)
                    .with_child(Expanded::new(1., info_column).finish())
                    .with_child(cta)
                    .finish(),
            )
            .with_padding(Padding::uniform(12.))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_border(Border::all(1.).with_border_fill(theme.outline()));

            if state.is_hovered() || state.is_clicked() {
                card = card.with_background(theme.surface_3());
            } else {
                card = card.with_background(theme.surface_1());
            }
            card.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(MarketplaceDirectoryAction::Install(
                source,
                entry_id.clone(),
            ))
        })
        .finish()
    }

    fn render_status_line(text: &str, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        Container::new(
            Text::new(
                text.to_string(),
                appearance.ui_font_family(),
                CARD_BODY_FONT_SIZE,
            )
            .with_color(blended_colors::text_sub(theme, theme.surface_2()))
            .with_selectable(false)
            .finish(),
        )
        .with_margin_bottom(8.)
        .finish()
    }

    fn render_content(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let visible = self.visible_entries(app);

        let mut column = Flex::column().with_main_axis_size(MainAxisSize::Max);

        if self.any_loading() {
            column =
                column.with_child(Self::render_status_line("Searching directories…", appearance));
        }
        for error in self.errors() {
            column = column.with_child(Self::render_status_line(error, appearance));
        }

        let mut cards = Wrap::row()
            .with_spacing(CARD_SPACING)
            .with_run_spacing(CARD_SPACING);
        for (index, (source, entry)) in visible.iter().enumerate() {
            let mouse_state = self
                .card_mouse_states
                .get(index)
                .cloned()
                .unwrap_or_default();
            let card = Self::render_card(*source, entry, mouse_state, appearance);
            cards = cards.with_child(ConstrainedBox::new(card).with_width(CARD_WIDTH).finish());
        }

        if visible.is_empty() && !self.any_loading() {
            column = column.with_child(Self::render_status_line(
                "No results. Try a different search or section.",
                appearance,
            ));
        } else {
            column = column
                .with_child(Shrinkable::new(1., Clipped::new(cards.finish()).finish()).finish());
        }

        Container::new(column.finish())
            .with_padding_left(20.)
            .with_padding_right(20.)
            .with_padding_top(16.)
            .with_padding_bottom(16.)
            .finish()
    }
}

impl Entity for MarketplaceDirectoryView {
    type Event = MarketplaceDirectoryEvent;
}

impl View for MarketplaceDirectoryView {
    fn ui_name() -> &'static str {
        "MarketplaceDirectoryView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        // Full-bleed pane content: left rail + card grid filling the canvas.
        let body = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(
                ConstrainedBox::new(self.render_nav(appearance))
                    .with_width(NAV_WIDTH)
                    .finish(),
            )
            .with_child(Expanded::new(1., self.render_content(appearance, app)).finish())
            .finish();

        Container::new(body)
            .with_background(theme.background())
            .finish()
    }
}

impl TypedActionView for MarketplaceDirectoryView {
    type Action = MarketplaceDirectoryAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            MarketplaceDirectoryAction::SelectSection(index) => {
                if let Some(section) = Section::ALL.get(*index) {
                    self.selected_section = *section;
                    ctx.notify();
                }
            }
            MarketplaceDirectoryAction::SelectInstallSpace(index) => {
                if let Some((space, _, _)) = self.space_options.get(*index) {
                    self.install_space = *space;
                    ctx.notify();
                }
            }
            MarketplaceDirectoryAction::Install(source, entry_id) => {
                let entry_id = entry_id.clone();
                self.install(*source, &entry_id, ctx);
            }
            MarketplaceDirectoryAction::OpenCustomize => {
                ctx.dispatch_typed_action(&WorkspaceAction::ShowSettings as &dyn warpui::Action);
            }
            MarketplaceDirectoryAction::OpenDocumentation => {
                ctx.open_url(DOCUMENTATION_URL);
            }
        }
    }
}

impl BackingView for MarketplaceDirectoryView {
    type PaneHeaderOverflowMenuAction = MarketplaceHeaderAction;
    type CustomAction = MarketplaceHeaderCustomAction;
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        _action: &Self::PaneHeaderOverflowMenuAction,
        _ctx: &mut ViewContext<Self>,
    ) {
        // No overflow menu items are registered.
    }

    fn handle_custom_action(
        &mut self,
        _custom_action: &Self::CustomAction,
        _ctx: &mut ViewContext<Self>,
    ) {
        // No custom header actions are registered.
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(MarketplaceDirectoryEvent::Pane(PaneEvent::Close));
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus(ctx);
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> HeaderContent {
        HeaderContent::Standard(StandardHeader {
            title: MARKETPLACE_HEADER_TEXT.to_string(),
            title_secondary: None,
            title_style: None,
            title_clip_config: ClipConfig::start(),
            title_max_width: None,
            left_of_title: None,
            right_of_title: None,
            left_of_overflow: None,
            options: StandardHeaderOptions::default(),
        })
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}
