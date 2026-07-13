//! Modal for importing MCP servers from Cursor configuration files
//! (`~/.cursor/mcp.json` and the current project's `.cursor/mcp.json`).
//!
//! Scanning reuses the exact parse path used by the file-based MCP watcher
//! ([`crate::ai::mcp::file_mcp_watcher::parse_mcp_config_file`] with
//! [`MCPProvider::Cursor`]), so imported servers behave identically to
//! detected ones. Env/header values are already templatized by
//! `ParsedTemplatableMCPServerResult::parse_result`, so secret values are
//! rendered masked and imported as installation variable values.

use std::collections::HashMap;
use std::path::PathBuf;

use warp_core::ui::icons::Icon;
use warpui::elements::{
    Align, Border, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty,
    Flex, Hoverable, MainAxisAlignment, MouseStateHandle, Padding, ParentElement, Radius,
    Shrinkable, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::Keystroke;
use warpui::platform::Cursor;
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

#[cfg(feature = "local_fs")]
use crate::ai::mcp::file_mcp_watcher::parse_mcp_config_file;
use crate::ai::mcp::templatable_installation::VariableValue;
use crate::ai::mcp::{
    MCPProvider, ParsedTemplatableMCPServerResult, ServerOrigin, TemplatableMCPServer,
    TemplatableMCPServerManager,
};
use crate::appearance::Appearance;
use crate::settings_view::mcp_servers::style::{
    INSTALLATION_MODAL_BUTTON_GAP, INSTALLATION_MODAL_PADDING,
    INSTALLATION_MODAL_TITLE_VERTICAL_SPACING,
};
use crate::ui_components::blended_colors;
use crate::view_components::action_button::{
    ActionButton, KeystrokeSource, NakedTheme, PrimaryTheme,
};

/// Mask shown in place of templatized env/header variable values.
const MASKED_VALUE: &str = "••••••••";
const SECRETS_WARNING: &str =
    "Environment and header values (including secrets) will be stored in Bang's MCP storage.";
const EMPTY_SCAN_TEXT: &str = "No MCP servers were found in your other tools' configurations.";
const SCANNING_TEXT: &str = "Scanning for MCP servers…";

pub enum ImportFromCursorModalBodyEvent {
    Cancel,
    /// Import the selected servers: `(template, variable_values)` pairs ready
    /// for `create_templatable_mcp_server` + `install_from_template`.
    Import(Vec<(TemplatableMCPServer, HashMap<String, VariableValue>)>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportFromCursorModalBodyAction {
    Cancel,
    Import,
    ToggleServer(usize),
}

/// A single importable server candidate produced by scanning Cursor configs.
pub struct CursorImportCandidate {
    pub parse_result: ParsedTemplatableMCPServerResult,
    /// The config file the server was found in.
    pub source_path: PathBuf,
    /// Whether an equivalent server is already installed in Warp.
    pub already_installed: bool,
    /// Whether the row's checkbox is checked. Ignored when `already_installed`.
    pub selected: bool,
    checkbox_mouse_state: MouseStateHandle,
}

impl CursorImportCandidate {
    fn new(
        parse_result: ParsedTemplatableMCPServerResult,
        source_path: PathBuf,
        already_installed: bool,
    ) -> Self {
        Self {
            parse_result,
            source_path,
            // New (not-yet-imported) servers are selected by default.
            selected: !already_installed,
            already_installed,
            checkbox_mouse_state: Default::default(),
        }
    }

    fn name(&self) -> &str {
        &self.parse_result.templatable_mcp_server.name
    }

    fn template(&self) -> &TemplatableMCPServer {
        &self.parse_result.templatable_mcp_server
    }
}

/// Returns a one-line summary of the server's transport: the command line for
/// stdio servers, or the URL for remote servers.
pub(crate) fn summary_for_template(template: &TemplatableMCPServer) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(&template.template.json).ok()?;
    let server = value.get(&template.name)?;
    if let Some(command) = server.get("command").and_then(|c| c.as_str()) {
        let args = server
            .get("args")
            .and_then(|a| a.as_array())
            .map(|args| {
                args.iter()
                    .filter_map(|a| a.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        if args.is_empty() {
            Some(command.to_owned())
        } else {
            Some(format!("{command} {args}"))
        }
    } else {
        server
            .get("url")
            .and_then(|u| u.as_str())
            .map(|u| u.to_owned())
    }
}

/// Returns `true` if a candidate server matches an already-installed or saved
/// server, either by (case-insensitive) name or by template JSON equality.
///
/// `existing` holds `(name, template_json)` pairs for every known template and
/// installation. Template JSON is compared structurally (parsed
/// [`serde_json::Value`]s) so formatting and key ordering differences don't
/// defeat the match.
pub(crate) fn is_already_installed(
    candidate_name: &str,
    candidate_template_json: &str,
    existing: &[(String, String)],
) -> bool {
    let candidate_name_lower = candidate_name.to_lowercase();
    let candidate_value: Option<serde_json::Value> =
        serde_json::from_str(candidate_template_json).ok();

    existing.iter().any(|(name, template_json)| {
        if name.to_lowercase() == candidate_name_lower {
            return true;
        }
        match (
            &candidate_value,
            serde_json::from_str::<serde_json::Value>(template_json).ok(),
        ) {
            (Some(candidate), Some(existing)) => candidate == &existing,
            _ => false,
        }
    })
}

/// Deduplicates candidates that appear in multiple scanned configs (e.g. the
/// same server defined both globally and in the project config). The first
/// occurrence (scan order: home, then project) wins. Comparison uses the same
/// name-or-template matching as [`is_already_installed`].
fn is_duplicate_of_existing_candidates(
    candidates: &[CursorImportCandidate],
    parse_result: &ParsedTemplatableMCPServerResult,
) -> bool {
    let existing: Vec<(String, String)> = candidates
        .iter()
        .map(|c| (c.name().to_owned(), c.template().template.json.clone()))
        .collect();
    is_already_installed(
        &parse_result.templatable_mcp_server.name,
        &parse_result.templatable_mcp_server.template.json,
        &existing,
    )
}

pub struct ImportFromCursorModalBody {
    candidates: Vec<CursorImportCandidate>,
    scanning: bool,
    cancel_button: ViewHandle<ActionButton>,
    import_button: ViewHandle<ActionButton>,
    close_button_mouse_state: MouseStateHandle,
}

impl ImportFromCursorModalBody {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let cancel_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Cancel", NakedTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(ImportFromCursorModalBodyAction::Cancel);
            })
        });

        let enter_keystroke = Keystroke::parse("enter").expect("valid keystroke");
        let import_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new("Import", PrimaryTheme)
                .with_keybinding(KeystrokeSource::Fixed(enter_keystroke), ctx)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(ImportFromCursorModalBodyAction::Import);
                })
        });

        Self {
            candidates: Vec::new(),
            scanning: false,
            cancel_button,
            import_button,
            close_button_mouse_state: Default::default(),
        }
    }

    /// Clears any previous scan results and scans the given `(provider, path)`
    /// config targets asynchronously, using the same parse path as the
    /// file-based MCP watcher. Each target is parsed with its own provider so a
    /// single scan can span multiple tools (Claude, Codex, Agents, Cursor, …).
    pub fn begin_scan(
        &mut self,
        scan_targets: Vec<(MCPProvider, PathBuf)>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.candidates.clear();
        cfg_if::cfg_if! {
            if #[cfg(feature = "local_fs")] {
                self.scanning = true;
                ctx.spawn(
                    async move {
                        let mut results = Vec::new();
                        for (provider, path) in scan_targets {
                            let servers = parse_mcp_config_file(&path, provider).await;
                            results.push((path, servers));
                        }
                        results
                    },
                    |me, results, ctx| {
                        me.apply_scan_results(results, ctx);
                    },
                );
            } else {
                let _ = (scan_targets, ctx);
                self.scanning = false;
            }
        }
        ctx.notify();
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn apply_scan_results(
        &mut self,
        results: Vec<(PathBuf, Vec<ParsedTemplatableMCPServerResult>)>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Gather `(name, template_json)` pairs for everything Warp already knows
        // about, so already-imported servers can be flagged.
        let manager = TemplatableMCPServerManager::as_ref(ctx);
        let mut existing: Vec<(String, String)> = manager
            .get_all_templatable_mcp_servers()
            .iter()
            .map(|template| (template.name.clone(), template.template.json.clone()))
            .collect();
        existing.extend(
            manager
                .get_installed_templatable_servers()
                .values()
                .map(|installation| {
                    let template = installation.templatable_mcp_server();
                    (template.name.clone(), template.template.json.clone())
                }),
        );

        self.candidates.clear();
        for (source_path, servers) in results {
            for parse_result in servers {
                if is_duplicate_of_existing_candidates(&self.candidates, &parse_result) {
                    continue;
                }
                // Imported servers carry CursorImport provenance.
                let parse_result = parse_result.with_origin(ServerOrigin::CursorImport);
                let already_installed = is_already_installed(
                    &parse_result.templatable_mcp_server.name,
                    &parse_result.templatable_mcp_server.template.json,
                    &existing,
                );
                self.candidates.push(CursorImportCandidate::new(
                    parse_result,
                    source_path.clone(),
                    already_installed,
                ));
            }
        }
        self.candidates
            .sort_by_key(|candidate| candidate.name().to_lowercase());
        self.scanning = false;
        ctx.notify();
    }

    /// Returns the `(template, variable_values)` pairs for the checked,
    /// not-already-installed candidates.
    fn selected_servers(&self) -> Vec<(TemplatableMCPServer, HashMap<String, VariableValue>)> {
        self.candidates
            .iter()
            .filter(|candidate| candidate.selected && !candidate.already_installed)
            .map(|candidate| {
                (
                    candidate.template().clone(),
                    candidate.parse_result.variable_values.clone(),
                )
            })
            .collect()
    }

    fn process_import(&mut self, ctx: &mut ViewContext<Self>) {
        let selected = self.selected_servers();
        if selected.is_empty() {
            return;
        }
        ctx.emit(ImportFromCursorModalBodyEvent::Import(selected));
    }

    fn render_title(
        appearance: &Appearance,
        close_button_mouse_state: MouseStateHandle,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let cursor_icon = ConstrainedBox::new(
            Icon::Import
                .to_warpui_icon(theme.active_ui_text_color())
                .finish(),
        )
        .with_width(24.)
        .with_height(24.)
        .finish();

        let title = Text::new(
            "Import MCP servers".to_string(),
            appearance.ui_font_family(),
            appearance.header_font_size(),
        )
        .with_color(theme.active_ui_text_color().into())
        .with_style(Properties::default().weight(Weight::Bold))
        .finish();

        // 'X' icon for closing the modal.
        let escape_icon = Shrinkable::new(
            1.,
            Align::new(
                Hoverable::new(close_button_mouse_state, |state| {
                    let mut icon = Container::new(
                        ConstrainedBox::new(
                            Icon::X
                                .to_warpui_icon(theme.active_ui_text_color())
                                .finish(),
                        )
                        .with_width(16.)
                        .with_height(16.)
                        .finish(),
                    )
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .with_padding(Padding::uniform(2.));
                    if state.is_hovered() {
                        icon = icon.with_background(appearance.theme().surface_2());
                    }
                    icon.finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(ImportFromCursorModalBodyAction::Cancel)
                })
                .finish(),
            )
            .right()
            .finish(),
        )
        .finish();

        let title_row = Flex::row()
            .with_children(vec![cursor_icon, title, escape_icon])
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_spacing(8.)
            .finish();

        Container::new(title_row)
            .with_margin_bottom(INSTALLATION_MODAL_TITLE_VERTICAL_SPACING)
            .finish()
    }

    fn render_candidate_row(
        &self,
        index: usize,
        candidate: &CursorImportCandidate,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let sub_color = blended_colors::text_sub(theme, theme.surface_1());

        let mut label_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(2.);

        // Server name.
        label_column.add_child(
            Text::new_inline(
                candidate.name().to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(theme.active_ui_text_color().into())
            .with_style(Properties::default().weight(Weight::Bold))
            .finish(),
        );

        // Command or URL summary.
        if let Some(summary) = summary_for_template(candidate.template()) {
            label_column.add_child(
                Text::new_inline(
                    summary,
                    appearance.ui_font_family(),
                    appearance.ui_font_size() * 0.9,
                )
                .with_color(sub_color)
                .finish(),
            );
        }

        // Masked env/header template variables.
        for variable in &candidate.template().template.variables {
            label_column.add_child(
                Text::new_inline(
                    format!("{} = {MASKED_VALUE}", variable.key),
                    appearance.ui_font_family(),
                    appearance.ui_font_size() * 0.9,
                )
                .with_color(sub_color)
                .finish(),
            );
        }

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(INSTALLATION_MODAL_BUTTON_GAP);

        if candidate.already_installed {
            row.add_child(
                Text::new_inline(
                    "Already imported".to_string(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size() * 0.9,
                )
                .with_color(theme.disabled_ui_text_color().into())
                .finish(),
            );
        } else {
            let checkbox = appearance
                .ui_builder()
                .checkbox(candidate.checkbox_mouse_state.clone(), Some(14.))
                .check(candidate.selected)
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(ImportFromCursorModalBodyAction::ToggleServer(index))
                })
                .finish();
            row.add_child(checkbox);
        }

        row.add_child(label_column.finish());

        Container::new(row.finish())
            .with_border(Border::bottom(1.).with_border_fill(theme.outline()))
            .with_padding(Padding::uniform(0.).with_vertical(8.))
            .finish()
    }

    fn render_body_content(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let sub_color = blended_colors::text_sub(theme, theme.surface_1());

        if self.scanning {
            return Text::new(
                SCANNING_TEXT.to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(sub_color)
            .finish();
        }

        if self.candidates.is_empty() {
            return Text::new(
                EMPTY_SCAN_TEXT.to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(sub_color)
            .finish();
        }

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        for (index, candidate) in self.candidates.iter().enumerate() {
            column.add_child(self.render_candidate_row(index, candidate, appearance));
        }

        // Warn about secret storage when any importable candidate carries
        // templatized env/header values.
        let has_variables = self.candidates.iter().any(|candidate| {
            !candidate.already_installed && !candidate.template().template.variables.is_empty()
        });
        if has_variables {
            column.add_child(
                Container::new(
                    Text::new(
                        SECRETS_WARNING.to_string(),
                        appearance.ui_font_family(),
                        appearance.ui_font_size() * 0.9,
                    )
                    .with_color(sub_color)
                    .finish(),
                )
                .with_margin_top(INSTALLATION_MODAL_TITLE_VERTICAL_SPACING)
                .finish(),
            );
        }

        column.finish()
    }

    fn render_buttons_row(&self, appearance: &Appearance) -> Box<dyn Element> {
        let spacer = Shrinkable::new(1., Container::new(Empty::new().finish()).finish()).finish();

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(spacer)
            .with_child(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(
                        Container::new(ChildView::new(&self.cancel_button).finish())
                            .with_margin_right(INSTALLATION_MODAL_BUTTON_GAP)
                            .finish(),
                    )
                    .with_child(
                        Container::new(ChildView::new(&self.import_button).finish()).finish(),
                    )
                    .finish(),
            )
            .finish();

        Container::new(row)
            .with_border(Border::top(1.).with_border_fill(appearance.theme().outline()))
            .with_uniform_padding(INSTALLATION_MODAL_PADDING)
            .finish()
    }
}

impl Entity for ImportFromCursorModalBody {
    type Event = ImportFromCursorModalBodyEvent;
}

impl View for ImportFromCursorModalBody {
    fn ui_name() -> &'static str {
        "ImportFromCursorModalBody"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.import_button);
        }
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(ctx);

        let mut form_column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        form_column.add_child(Self::render_title(
            appearance,
            self.close_button_mouse_state.clone(),
        ));
        form_column.add_child(self.render_body_content(appearance));

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(form_column.finish())
                    .with_uniform_padding(INSTALLATION_MODAL_PADDING)
                    .finish(),
            )
            .with_child(self.render_buttons_row(appearance))
            .finish()
    }
}

impl TypedActionView for ImportFromCursorModalBody {
    type Action = ImportFromCursorModalBodyAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ImportFromCursorModalBodyAction::Cancel => {
                ctx.emit(ImportFromCursorModalBodyEvent::Cancel)
            }
            ImportFromCursorModalBodyAction::Import => self.process_import(ctx),
            ImportFromCursorModalBodyAction::ToggleServer(index) => {
                if let Some(candidate) = self.candidates.get_mut(*index) {
                    if !candidate.already_installed {
                        candidate.selected = !candidate.selected;
                        ctx.notify();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "import_from_cursor_modal_tests.rs"]
mod tests;
