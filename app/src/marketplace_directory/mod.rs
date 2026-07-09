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

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;

use warpui::elements::{
    Border, ChildView, Clipped, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
    Container, CornerRadius, CrossAxisAlignment, Expanded, Fill, Flex, Hoverable, MainAxisSize,
    MouseStateHandle, Padding, ParentElement, Radius, ScrollbarWidth, Text, Wrap,
};
use warpui::fonts::Weight;
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, EventContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use std::collections::HashSet;

use ai::skills::{home_skills_path, SkillProvider};

use crate::ai::execution_profiles::AIExecutionProfile;
use crate::ai::facts::{AIFact, AIMemory};
use crate::ai::mcp::parsing::ParsedTemplatableMCPServerResult;
use crate::ai::mcp::{ServerOrigin, TemplatableMCPServerManager};
use crate::appearance::Appearance;
use crate::cloud_object::{CloudObjectEventEntrypoint, Owner, Space};
use crate::editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions};
use crate::marketplace_plugins::{CloudMarketplacePluginModel, MarketplacePlugin, PluginSource};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view::{self, HeaderContent, StandardHeader, StandardHeaderOptions};
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent};
use crate::server::cloud_objects::update_manager::{InitiatedBy, UpdateManager};
use crate::server::ids::ClientId;
use crate::server::server_api::marketplace::{
    MarketplaceComponentType, MarketplaceEntryKind, MarketplacePluginComponentFile,
    MarketplaceSearchEntry, MarketplaceSourceKind, ResolvedMarketplacePlugin,
};
use crate::server::server_api::ServerApiProvider;
use crate::ui_components::avatar::{Avatar, AvatarContent};
use crate::ui_components::blended_colors;
use crate::ui_components::buttons::icon_button;
use crate::ui_components::icons::Icon;
use crate::view_components::DismissibleToast;
use crate::workflows::workflow::Workflow;
use crate::workspace::WorkspaceAction;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::ToastStack;

const NAV_WIDTH: f32 = 216.;
const CARD_WIDTH: f32 = 300.;
const CARD_SPACING: f32 = 8.;
const NAV_FONT_SIZE: f32 = 13.;
const CARD_TITLE_FONT_SIZE: f32 = 14.;
const CARD_BODY_FONT_SIZE: f32 = 12.;

const DOCUMENTATION_URL: &str = "https://modelcontextprotocol.io/docs";

/// Header text for the marketplace ("Connectors") pane.
pub const MARKETPLACE_HEADER_TEXT: &str = "Connectors";

/// The left-rail sections. `All` merges every directory; `Popular` shows the
/// most-installed entries in your team; the rest filter to one backing source.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    All,
    Popular,
    Cursor,
    Org,
    McpRegistry,
}

impl Section {
    const ALL: &'static [Section] = &[
        Section::All,
        Section::Popular,
        Section::Cursor,
        Section::Org,
        Section::McpRegistry,
    ];

    fn label(self) -> &'static str {
        match self {
            Section::All => "All",
            Section::Popular => "Popular in your team",
            Section::Cursor => "Cursor",
            Section::Org => "Your org",
            Section::McpRegistry => "MCP servers",
        }
    }
}

/// How the visible cards are ordered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortMode {
    /// The server's merged relevance order (default).
    Relevance,
    /// Alphabetical by title.
    Name,
    /// Most-installed-in-your-team first; entries without a count sort last.
    Popular,
}

impl SortMode {
    const ALL: &'static [SortMode] = &[SortMode::Relevance, SortMode::Name, SortMode::Popular];

    fn label(self) -> &'static str {
        match self {
            SortMode::Relevance => "Relevance",
            SortMode::Name => "Name",
            SortMode::Popular => "Popularity",
        }
    }
}

/// The sources fetched by the rail search, in merge/display order.
///
/// Open VSX (VS Code extension registry) is intentionally excluded: Bang has no
/// VS Code extension host, so `.vsix` extensions can't run here, and none of
/// them expose an MCP server we could extract.
const SOURCES: [MarketplaceSourceKind; 3] = [
    MarketplaceSourceKind::Cursor,
    MarketplaceSourceKind::Org,
    MarketplaceSourceKind::McpRegistry,
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
    SelectCategory(usize),
    SelectSort(usize),
    SelectInstallSpace(usize),
    /// Install the entry with this id from this source.
    Install(MarketplaceSourceKind, String),
    OpenCustomize,
    OpenDocumentation,
    /// Close the Connectors pane.
    Close,
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
    /// Optional domain-category filter (from entries' `category` field), shown
    /// as a CATEGORIES rail below BROWSE. `None` means "all categories".
    selected_category: Option<String>,
    sort_mode: SortMode,
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
    close_button_mouse_state: MouseStateHandle,
    section_mouse_states: Vec<MouseStateHandle>,
    sort_mouse_states: Vec<MouseStateHandle>,
    category_mouse_states: Vec<MouseStateHandle>,
    card_mouse_states: Vec<MouseStateHandle>,
    /// Vertical scroll for the card grid so long directories don't clip.
    content_scroll_state: ClippedScrollStateHandle,
    /// Vertical scroll for the left rail so many categories don't clip the
    /// sections below them.
    nav_scroll_state: ClippedScrollStateHandle,
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
            selected_category: None,
            sort_mode: SortMode::Relevance,
            install_space: Space::Personal,
            space_options: Vec::new(),
            search_editor,
            source_states: Default::default(),
            fetched_query: String::new(),
            customize_mouse_state: Default::default(),
            documentation_mouse_state: Default::default(),
            close_button_mouse_state: Default::default(),
            section_mouse_states: Section::ALL
                .iter()
                .map(|_| MouseStateHandle::default())
                .collect(),
            sort_mouse_states: SortMode::ALL
                .iter()
                .map(|_| MouseStateHandle::default())
                .collect(),
            category_mouse_states: Vec::new(),
            card_mouse_states: Vec::new(),
            content_scroll_state: ClippedScrollStateHandle::default(),
            nav_scroll_state: ClippedScrollStateHandle::default(),
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
                    me.resize_category_mouse_states();
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

    fn resize_category_mouse_states(&mut self) {
        let total = self.available_categories().len();
        while self.category_mouse_states.len() < total {
            self.category_mouse_states.push(MouseStateHandle::default());
        }
    }

    /// Distinct, human-readable domain categories across all loaded entries,
    /// sorted alphabetically. Powers the CATEGORIES rail and its filter.
    fn available_categories(&self) -> Vec<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut categories: Vec<String> = Vec::new();
        for state in &self.source_states {
            if let SourceState::Loaded(entries) = state {
                for entry in entries {
                    let Some(raw) = entry.category.as_deref() else {
                        continue;
                    };
                    let label = prettify_category(raw);
                    if label.is_empty() {
                        continue;
                    }
                    if seen.insert(label.to_lowercase()) {
                        categories.push(label);
                    }
                }
            }
        }
        categories.sort_by_key(|c| c.to_lowercase());
        categories
    }

    /// The entries visible under the current section + live text filter, as
    /// `(source, entry)` pairs in merged display order.
    fn visible_entries(
        &self,
        app: &AppContext,
    ) -> Vec<(MarketplaceSourceKind, &MarketplaceSearchEntry)> {
        let term = self.search_term(app).to_lowercase();
        let wanted_source = match self.selected_section {
            Section::All | Section::Popular => None,
            Section::Cursor => Some(MarketplaceSourceKind::Cursor),
            Section::Org => Some(MarketplaceSourceKind::Org),
            Section::McpRegistry => Some(MarketplaceSourceKind::McpRegistry),
        };
        let popular_only = self.selected_section == Section::Popular;
        let wanted_category = self.selected_category.as_deref().map(str::to_lowercase);

        let mut visible = Vec::new();
        for source in SOURCES {
            if wanted_source.is_some_and(|wanted| wanted != source) {
                continue;
            }
            if let SourceState::Loaded(entries) = &self.source_states[source_index(source)] {
                for entry in entries {
                    if popular_only && entry.install_count.unwrap_or(0) <= 0 {
                        continue;
                    }
                    if let Some(wanted) = &wanted_category {
                        let matches_category = entry
                            .category
                            .as_deref()
                            .is_some_and(|c| prettify_category(c).to_lowercase() == *wanted);
                        if !matches_category {
                            continue;
                        }
                    }
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

        // The Popular section always ranks by install count; elsewhere honor
        // the chosen sort. Relevance keeps the server's merged order.
        let effective_sort = if popular_only {
            SortMode::Popular
        } else {
            self.sort_mode
        };
        match effective_sort {
            SortMode::Relevance => {}
            SortMode::Name => {
                visible.sort_by(|a, b| a.1.title.to_lowercase().cmp(&b.1.title.to_lowercase()));
            }
            SortMode::Popular => {
                visible.sort_by(|a, b| {
                    b.1.install_count
                        .unwrap_or(0)
                        .cmp(&a.1.install_count.unwrap_or(0))
                        .then_with(|| a.1.title.to_lowercase().cmp(&b.1.title.to_lowercase()))
                });
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
            MarketplaceEntryKind::Mcp => {
                self.install_mcp(source, &entry, ctx);
                self.report_install(source, &entry, ctx);
            }
            MarketplaceEntryKind::Plugin if Self::is_cursor_plugin(&entry) => {
                // Multi-component `.cursor-plugin` entry: fetch the full
                // component bodies on demand, then install each into its native
                // Bang home.
                self.install_cursor_plugin(source, entry, ctx);
            }
            MarketplaceEntryKind::Plugin => {
                self.install_plugin(source, &entry, ctx);
                self.report_install(source, &entry, ctx);
            }
        }
    }

    /// A Cursor `.cursor-plugin` entry carries a component summary; Open VSX
    /// plugins instead carry an `extension_id` / `bundle_url` and no components.
    fn is_cursor_plugin(entry: &MarketplaceSearchEntry) -> bool {
        entry.components.is_some() && entry.extension_id.is_none()
    }

    fn origin_for_source(source: MarketplaceSourceKind) -> ServerOrigin {
        match source {
            MarketplaceSourceKind::Org => ServerOrigin::OrgMarketplace,
            MarketplaceSourceKind::Cursor
            | MarketplaceSourceKind::McpRegistry
            | MarketplaceSourceKind::OpenVsx => ServerOrigin::Registry,
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
        if self.install_mcp_template(source, template_json, ctx) == 0 {
            self.show_toast("Couldn't parse this entry's MCP config.".to_owned(), ctx);
        } else {
            self.show_toast(format!("Added {} to your MCP servers.", entry.title), ctx);
        }
    }

    /// Installs every server in a `{"mcpServers": {...}}` template into the
    /// selected space. Returns how many servers were installed (0 on parse
    /// failure), so callers can compose an install summary.
    fn install_mcp_template(
        &mut self,
        source: MarketplaceSourceKind,
        template_json: &str,
        ctx: &mut ViewContext<Self>,
    ) -> usize {
        let parsed_servers = match ParsedTemplatableMCPServerResult::from_user_json(template_json) {
            Ok(parsed) if !parsed.is_empty() => parsed,
            _ => return 0,
        };

        let space = self.install_space;
        let origin = Self::origin_for_source(source);
        let count = parsed_servers.len();
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
        count
    }

    /// Resolves a `.cursor-plugin` entry's full component contents from the
    /// backend, then installs each component. Async because the bodies are
    /// fetched on demand (kept out of the directory listing).
    fn install_cursor_plugin(
        &mut self,
        source: MarketplaceSourceKind,
        entry: MarketplaceSearchEntry,
        ctx: &mut ViewContext<Self>,
    ) {
        let entry_id = entry.entry_id.inner().to_owned();
        let client = ServerApiProvider::handle(ctx)
            .as_ref(ctx)
            .get_marketplace_client();
        self.show_toast(format!("Installing {}…", entry.title), ctx);
        ctx.spawn(
            async move {
                let resolved = client.resolve_marketplace_plugin(source, entry_id).await;
                (entry, resolved)
            },
            move |me, (entry, resolved), ctx| match resolved {
                Ok(resolved) => me.apply_resolved_plugin(source, entry, resolved, ctx),
                Err(err) => me.show_toast(format!("Couldn't install {}: {err}", entry.title), ctx),
            },
        );
    }

    /// Materializes each resolved component into its native Bang home: MCP
    /// servers, rules (AIFact), commands (Workflow), agents (execution
    /// profile), and skills (files under `~/.agents/skills`). Hooks have no
    /// Bang equivalent and are reported as skipped.
    fn apply_resolved_plugin(
        &mut self,
        source: MarketplaceSourceKind,
        entry: MarketplaceSearchEntry,
        resolved: ResolvedMarketplacePlugin,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(owner) = UserWorkspaces::as_ref(ctx).space_to_owner(self.install_space, ctx)
        else {
            self.show_toast("Couldn't resolve the install destination.".to_owned(), ctx);
            return;
        };

        let mut summary: Vec<String> = Vec::new();

        if let Some(template_json) = resolved.mcp_template_json.as_deref() {
            let installed = self.install_mcp_template(source, template_json, ctx);
            if installed > 0 {
                summary.push(pluralize(installed, "MCP server"));
            }
        }

        let mut rules = 0usize;
        let mut commands = 0usize;
        let mut agents = 0usize;
        let mut skipped_hooks = 0usize;
        let mut skill_files: Vec<&MarketplacePluginComponentFile> = Vec::new();

        for file in &resolved.files {
            match file.component_type {
                MarketplaceComponentType::Rule => {
                    self.create_rule(&file.name, &file.content, owner, ctx);
                    rules += 1;
                }
                MarketplaceComponentType::Command => {
                    self.create_command(&file.name, &file.content, owner, ctx);
                    commands += 1;
                }
                MarketplaceComponentType::Agent => {
                    self.create_agent_profile(&file.name, owner, ctx);
                    agents += 1;
                }
                MarketplaceComponentType::Skill => skill_files.push(file),
                MarketplaceComponentType::Hook => skipped_hooks += 1,
                // The MCP part is installed via `mcp_template_json` above.
                MarketplaceComponentType::McpServer => {}
            }
        }
        if rules > 0 {
            summary.push(pluralize(rules, "rule"));
        }
        if commands > 0 {
            summary.push(pluralize(commands, "command"));
        }
        if agents > 0 {
            summary.push(pluralize(agents, "agent"));
        }
        let skills = install_skill_files(&skill_files);
        if skills > 0 {
            summary.push(pluralize(skills, "skill"));
        }

        self.report_install(source, &entry, ctx);

        let mut message = if summary.is_empty() {
            format!("Installed {}.", entry.title)
        } else {
            format!("Installed {}: {}.", entry.title, summary.join(", "))
        };
        if skipped_hooks > 0 {
            message.push_str(&format!(
                " Skipped {} (hooks aren't supported in Bang yet).",
                pluralize(skipped_hooks, "hook")
            ));
        }
        self.show_toast(message, ctx);
    }

    /// Creates a Rule (AIFact) from a rule file body.
    fn create_rule(&self, name: &str, content: &str, owner: Owner, ctx: &mut ViewContext<Self>) {
        let ai_fact = AIFact::Memory(AIMemory {
            name: (!name.is_empty()).then(|| name.to_string()),
            content: content.to_string(),
            is_autogenerated: false,
            suggested_logging_id: None,
        });
        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            update_manager.create_ai_fact(ai_fact, ClientId::default(), owner, ctx);
        });
    }

    /// Creates an agent-mode Workflow (Cursor "command") from a command file.
    fn create_command(&self, name: &str, content: &str, owner: Owner, ctx: &mut ViewContext<Self>) {
        let workflow = Workflow::AgentMode {
            name: name.to_string(),
            query: content.to_string(),
            description: None,
            arguments: Vec::new(),
        };
        let client_id = ClientId::default();
        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            update_manager.create_workflow(
                workflow,
                owner,
                None,
                client_id,
                CloudObjectEventEntrypoint::ManagementUI,
                true,
                ctx,
            );
        });
    }

    /// Creates an AI execution profile (Cursor "agent") named after the file.
    /// Cursor agent files carry a prompt/persona we can't fully model yet, so
    /// we seed a default-permission profile the user can then tailor.
    fn create_agent_profile(&self, name: &str, owner: Owner, ctx: &mut ViewContext<Self>) {
        let profile = AIExecutionProfile {
            name: name.to_string(),
            is_default_profile: false,
            ..Default::default()
        };
        let client_id = ClientId::default();
        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            update_manager.create_ai_execution_profile(profile, client_id, owner, ctx);
        });
    }

    /// Fire-and-forget report of an install for the per-team popularity
    /// leaderboard. The backend derives the team from the caller's membership.
    fn report_install(
        &self,
        source: MarketplaceSourceKind,
        entry: &MarketplaceSearchEntry,
        ctx: &mut ViewContext<Self>,
    ) {
        let entry_id = entry.entry_id.inner().to_owned();
        let title = Some(entry.title.clone());
        let client = ServerApiProvider::handle(ctx)
            .as_ref(ctx)
            .get_marketplace_client();
        ctx.spawn(
            async move {
                let _ = client
                    .report_marketplace_install(source, entry_id, title, None)
                    .await;
            },
            |_me, _res, _ctx| {},
        );
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
        let mut column = Flex::column().with_main_axis_size(MainAxisSize::Min);

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

        let categories = self.available_categories();
        if !categories.is_empty() {
            column = column.with_child(Self::render_nav_heading("CATEGORIES", appearance));
            for (index, category) in categories.iter().enumerate() {
                let selected = self.selected_category.as_deref() == Some(category);
                let mouse_state = self
                    .category_mouse_states
                    .get(index)
                    .cloned()
                    .unwrap_or_default();
                column = column.with_child(Self::render_nav_item(
                    category,
                    selected,
                    mouse_state,
                    appearance,
                    move |ctx| {
                        ctx.dispatch_typed_action(MarketplaceDirectoryAction::SelectCategory(index))
                    },
                ));
            }
        }

        column = column.with_child(Self::render_nav_heading("SORT BY", appearance));
        for (index, mode) in SortMode::ALL.iter().enumerate() {
            let selected = *mode == self.sort_mode;
            column = column.with_child(Self::render_nav_item(
                mode.label(),
                selected,
                self.sort_mouse_states[index].clone(),
                appearance,
                move |ctx| ctx.dispatch_typed_action(MarketplaceDirectoryAction::SelectSort(index)),
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

        Container::new(
            ClippedScrollable::vertical(
                self.nav_scroll_state.clone(),
                column.finish(),
                ScrollbarWidth::Auto,
                theme.nonactive_ui_text_color().into(),
                theme.active_ui_text_color().into(),
                Fill::None,
            )
            .with_overlayed_scrollbar()
            .finish(),
        )
        .with_padding_left(8.)
        .with_padding_right(8.)
        .with_padding_top(12.)
        .with_border(Border::right(1.).with_border_fill(theme.outline()))
        .finish()
    }

    /// Short chip labels describing a plugin's category and component types.
    /// Retained for tests / potential detail views; cards no longer render it.
    #[cfg(test)]
    fn component_badges(entry: &MarketplaceSearchEntry) -> Vec<String> {
        let mut badges = Vec::new();
        if let Some(category) = entry.category.as_deref().filter(|c| !c.is_empty()) {
            badges.push(category.to_string());
        }
        if let Some(components) = &entry.components {
            for (count, label) in [
                (components.mcp_server_count, "MCP"),
                (components.rule_count, "Rules"),
                (components.skill_count, "Skills"),
                (components.agent_count, "Agents"),
                (components.command_count, "Commands"),
                (components.hook_count, "Hooks"),
            ] {
                if count > 0 {
                    badges.push(label.to_string());
                }
            }
        }
        badges
    }

    fn render_card(
        source: MarketplaceSourceKind,
        entry: &MarketplaceSearchEntry,
        mouse_state: MouseStateHandle,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let title_text = entry.title.clone();
        // Compact subtitle: publisher and source de-duplicated (avoids
        // "Cursor · Cursor"), plus install count when it's a team favorite.
        let mut subtitle_parts: Vec<String> = Vec::new();
        if let Some(publisher) = entry.publisher.as_deref().filter(|p| !p.is_empty()) {
            subtitle_parts.push(publisher.to_owned());
        }
        if !entry.source_label.is_empty()
            && entry.publisher.as_deref() != Some(entry.source_label.as_str())
        {
            subtitle_parts.push(entry.source_label.clone());
        }
        if let Some(installs) = entry.install_count.filter(|n| *n > 0) {
            subtitle_parts.push(format!(
                "{installs} install{}",
                if installs == 1 { "" } else { "s" }
            ));
        }
        let subtitle_text = subtitle_parts.join(" · ");
        let description_text: String = entry.description.chars().take(72).collect();
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
                    width: Some(28.),
                    height: Some(28.),
                    border_radius: Some(CornerRadius::with_all(Radius::Pixels(5.))),
                    font_family_id: Some(font_family),
                    font_weight: Some(Weight::Bold),
                    background: Some(theme.background().into()),
                    font_size: Some(14.),
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

            // Cursor-style compact card: no component-badge chip row. Component
            // details live on the plugin's resolve/detail view instead.
            let mut info_column = Flex::column().with_child(name);
            if !subtitle_text.is_empty() {
                info_column =
                    info_column.with_child(Container::new(subtitle).with_margin_top(2.).finish());
            }
            let info_column = info_column
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
                    .with_spacing(8.)
                    .with_child(avatar)
                    .with_child(Expanded::new(1., info_column).finish())
                    .with_child(cta)
                    .finish(),
            )
            .with_padding(Padding::uniform(10.))
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
        let theme = appearance.theme();
        let visible = self.visible_entries(app);

        let mut column = Flex::column().with_main_axis_size(MainAxisSize::Max);

        if self.any_loading() {
            column = column.with_child(Self::render_status_line(
                "Searching directories…",
                appearance,
            ));
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
            // Fill the remaining height with a vertical scroll so long
            // directories are reachable instead of clipped.
            column = column.with_child(
                Expanded::new(
                    1.,
                    ClippedScrollable::vertical(
                        self.content_scroll_state.clone(),
                        cards.finish(),
                        ScrollbarWidth::Auto,
                        theme.nonactive_ui_text_color().into(),
                        theme.active_ui_text_color().into(),
                        Fill::None,
                    )
                    .with_overlayed_scrollbar()
                    .finish(),
                )
                .finish(),
            );
        }

        Container::new(column.finish())
            .with_padding_left(20.)
            .with_padding_right(20.)
            .with_padding_top(16.)
            .with_padding_bottom(16.)
            .finish()
    }
}

/// Writes resolved skill files under `~/.agents/skills/<skill>/<path>` so the
/// SkillManager's file watcher discovers them (this is the Cursor-compatible,
/// highest-precedence skills home). Returns the count of distinct skill folders
/// written. Path segments are sanitized so a manifest can't escape the dir.
fn install_skill_files(files: &[&MarketplacePluginComponentFile]) -> usize {
    if files.is_empty() {
        return 0;
    }
    let Some(base) = home_skills_path(SkillProvider::Agents) else {
        return 0;
    };
    let mut installed: HashSet<String> = HashSet::new();
    for file in files {
        let Some(skill_dir) = sanitize_path_component(&file.name) else {
            continue;
        };
        let Some(rel_path) = sanitize_relative_path(&file.path) else {
            continue;
        };
        let dest = base.join(&skill_dir).join(&rel_path);
        let Some(parent) = dest.parent() else {
            continue;
        };
        if std::fs::create_dir_all(parent).is_err() {
            continue;
        }
        if std::fs::write(&dest, file.content.as_bytes()).is_ok() {
            installed.insert(skill_dir);
        }
    }
    installed.len()
}

/// A single, traversal-safe path segment (the skill folder name).
fn sanitize_path_component(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
    {
        return None;
    }
    Some(trimmed.to_string())
}

/// A traversal-safe relative path (may contain `/`), used for a file within a
/// skill folder.
fn sanitize_relative_path(path: &str) -> Option<std::path::PathBuf> {
    let mut out = std::path::PathBuf::new();
    for segment in path.trim().split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." || segment.contains('\\') {
            return None;
        }
        out.push(segment);
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

fn pluralize(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// Turns a raw category slug/label (e.g. `developer-tools`, `data_analytics`)
/// into a display label (`Developer Tools`, `Data Analytics`), capping length.
fn prettify_category(raw: &str) -> String {
    raw.trim()
        .split(|c: char| c == '-' || c == '_' || c.is_whitespace())
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(40)
        .collect()
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
                    // Sections and categories are independent axes; picking a
                    // section clears any active category filter.
                    self.selected_category = None;
                    ctx.notify();
                }
            }
            MarketplaceDirectoryAction::SelectCategory(index) => {
                if let Some(category) = self.available_categories().get(*index) {
                    // Toggle: clicking the active category clears the filter.
                    self.selected_category = if self.selected_category.as_deref() == Some(category)
                    {
                        None
                    } else {
                        Some(category.clone())
                    };
                    ctx.notify();
                }
            }
            MarketplaceDirectoryAction::SelectSort(index) => {
                if let Some(mode) = SortMode::ALL.get(*index) {
                    self.sort_mode = *mode;
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
            MarketplaceDirectoryAction::Close => {
                self.close(ctx);
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
        app: &AppContext,
    ) -> HeaderContent {
        let appearance = Appearance::as_ref(app);
        // Connectors opens as a full tab (not a split pane), so the framework's
        // built-in close button never shows. Add an always-visible X that closes
        // the pane.
        let close_button = icon_button(
            appearance,
            Icon::X,
            false,
            self.close_button_mouse_state.clone(),
        )
        .build()
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(MarketplaceDirectoryAction::Close);
        })
        .with_cursor(Cursor::PointingHand)
        .finish();

        HeaderContent::Standard(StandardHeader {
            title: MARKETPLACE_HEADER_TEXT.to_string(),
            title_secondary: None,
            title_style: None,
            title_clip_config: ClipConfig::start(),
            title_max_width: None,
            left_of_title: None,
            right_of_title: None,
            left_of_overflow: Some(close_button),
            options: StandardHeaderOptions::default(),
        })
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}
