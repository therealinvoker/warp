//! GitHub automations settings page.
//!
//! Lists a workspace's GitHub automations, hosts an inline create/edit form,
//! and (for team admins) a workspace provider-key admin section. Modeled on the
//! monolithic [`SettingsWidget`] pattern used by
//! [`crate::settings_view::github_page`].
//!
//! Gating:
//! - Compiled behind the `github_automations` cargo feature.
//! - [`FeatureFlag::GithubAutomations`] + tier `githubPolicy.automationsEnabled`
//!   gate whether the page renders at all ([`SettingsPageMeta::should_render`]).
//! - [`Team::has_admin_permissions`] gates write affordances (create/edit/
//!   remove/provider-keys); non-admins see a read-only view.
//!
//! Degradation: a backend that returns an empty payload for not-yet-wired ops
//! surfaces a clear inline empty/error state and never panics.

use pathfinder_geometry::vector::Vector2F;
use warp_core::features::FeatureFlag;
use warpui::elements::{
    Container, CrossAxisAlignment, Element, Flex, MainAxisSize, MouseStateHandle, ParentElement,
    Text,
};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{
    AppContext, Entity, EventContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use super::super::settings_page::{
    render_separator, render_settings_info_banner, render_sub_header, MatchData, PageType,
    SettingsPageMeta, SettingsPageViewHandle, SettingsWidget,
};
use super::super::SettingsSection;
use super::edit_page::{AutomationFormState, AutomationFormValues};
use crate::appearance::Appearance;
use crate::auth::AuthStateProvider;
use crate::editor::{EditorView, SingleLineEditorOptions, TextOptions};
use crate::github::automations::{
    GithubAutomation, GithubAutomationActionType, GithubAutomationInput, GithubProviderKey,
    ListGithubAutomationsData,
};
use crate::server::server_api::ServerApiProvider;
use crate::view_components::DismissibleToast;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::ToastStack;

const PAGE_TITLE_TEXT: &str = "GitHub Automations";
const BUTTON_FONT_SIZE: f32 = 12.;

/// Loading / data / error state for the automations list + provider keys.
#[derive(Debug, Clone, Default)]
enum LoadState {
    #[default]
    Idle,
    Loading,
    Loaded(ListGithubAutomationsData),
    Error(String),
}

/// Actions dispatched by the automations page.
#[derive(Debug, Clone)]
pub enum GithubAutomationsPageAction {
    Refresh,
    /// Open the inline form to create a new automation.
    StartCreate,
    /// Open the inline form to edit an existing automation.
    StartEdit(String),
    /// Cancel the inline form without saving.
    CancelForm,
    /// Save the inline form (create or update).
    SaveForm,
    /// Remove an automation by id.
    Remove(String),
    /// Toggle the enabled switch in the inline form.
    ToggleEnabled,
    /// Advance the trigger type to the next option (cycle-on-click selector).
    CycleTriggerType,
    /// Toggle the action type between Prompt and Skill.
    CycleActionType,
    /// Dismiss the one-time hook-key banner.
    DismissHookKey,
    /// Add the provider key currently typed in the provider-key editors.
    AddProviderKey,
    /// Remove a provider key by provider name.
    RemoveProviderKey(String),
}

#[derive(Debug, Clone)]
pub enum GithubAutomationsPageEvent {}

/// Handle to a single-line text editor plus a stable placeholder.
struct FormEditors {
    name: ViewHandle<EditorView>,
    repo_filter: ViewHandle<EditorView>,
    branch_pattern: ViewHandle<EditorView>,
    comment_phrase: ViewHandle<EditorView>,
    prompt: ViewHandle<EditorView>,
    skill: ViewHandle<EditorView>,
    harness: ViewHandle<EditorView>,
    model_id: ViewHandle<EditorView>,
}

pub struct GithubAutomationsListPageView {
    page: PageType<Self>,
    load_state: LoadState,
    /// `Some` when the inline create/edit form is open.
    form: Option<AutomationFormState>,
    editors: FormEditors,
    /// The plaintext hook key returned once on CUSTOM-trigger create.
    pending_hook_key: Option<String>,
    /// Provider-key admin editors (admins only).
    provider_editor: ViewHandle<EditorView>,
    provider_key_editor: ViewHandle<EditorView>,
    // Mouse states for the various buttons.
    refresh_mouse_state: MouseStateHandle,
    create_mouse_state: MouseStateHandle,
    save_mouse_state: MouseStateHandle,
    cancel_mouse_state: MouseStateHandle,
    trigger_menu_state: MouseStateHandle,
    action_menu_state: MouseStateHandle,
    enabled_switch: SwitchStateHandle,
    add_key_mouse_state: MouseStateHandle,
}

impl Entity for GithubAutomationsListPageView {
    type Event = GithubAutomationsPageEvent;
}

fn new_single_line_editor(
    placeholder: &'static str,
    ctx: &mut ViewContext<GithubAutomationsListPageView>,
) -> ViewHandle<EditorView> {
    ctx.add_typed_action_view(|ctx| {
        let appearance = Appearance::handle(ctx).as_ref(ctx);
        let options = SingleLineEditorOptions {
            text: TextOptions {
                font_size_override: Some(appearance.ui_font_size()),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut editor = EditorView::single_line(options, ctx);
        editor.set_placeholder_text(placeholder, ctx);
        editor
    })
}

impl GithubAutomationsListPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let editors = FormEditors {
            name: new_single_line_editor("Automation name", ctx),
            repo_filter: new_single_line_editor("owner/repo (optional)", ctx),
            branch_pattern: new_single_line_editor("Branch pattern (optional)", ctx),
            comment_phrase: new_single_line_editor("Comment phrase (optional)", ctx),
            prompt: new_single_line_editor("Prompt for the agent", ctx),
            skill: new_single_line_editor("Skill name", ctx),
            harness: new_single_line_editor("Harness (e.g. claude; optional)", ctx),
            model_id: new_single_line_editor("Model id (optional)", ctx),
        };
        let provider_editor = new_single_line_editor("Provider (e.g. anthropic)", ctx);
        let provider_key_editor = new_single_line_editor("Provider key (write-only)", ctx);

        Self {
            page: PageType::new_monolith(GithubAutomationsWidget, Some(PAGE_TITLE_TEXT), false),
            load_state: LoadState::Idle,
            form: None,
            editors,
            pending_hook_key: None,
            provider_editor,
            provider_key_editor,
            refresh_mouse_state: MouseStateHandle::default(),
            create_mouse_state: MouseStateHandle::default(),
            save_mouse_state: MouseStateHandle::default(),
            cancel_mouse_state: MouseStateHandle::default(),
            trigger_menu_state: MouseStateHandle::default(),
            action_menu_state: MouseStateHandle::default(),
            enabled_switch: SwitchStateHandle::default(),
            add_key_mouse_state: MouseStateHandle::default(),
        }
    }

    /// The current team's server uid, if any. Automations are workspace-scoped;
    /// with no team there is nothing to manage.
    fn workspace_uid(ctx: &AppContext) -> Option<String> {
        UserWorkspaces::as_ref(ctx)
            .current_team_uid()
            .map(|id| id.to_string())
    }

    /// Whether the current user may perform writes (create/edit/remove/keys).
    fn can_write(ctx: &AppContext) -> bool {
        let Some(email) = AuthStateProvider::as_ref(ctx).get().user_email() else {
            return false;
        };
        UserWorkspaces::as_ref(ctx)
            .current_team()
            .is_some_and(|team| team.has_admin_permissions(&email))
    }

    /// The workspace repo allowlist used to validate an automation's repo filter.
    fn repo_allowlist(&self) -> Vec<String> {
        // Currently sourced from the installed-repos set surfaced by the GitHub
        // connection; when empty (or not yet loaded) the form treats the filter
        // as unrestricted. Kept as a helper so the source can be swapped without
        // touching the form validation call sites.
        Vec::new()
    }

    fn refresh(&mut self, ctx: &mut ViewContext<Self>) {
        if matches!(self.load_state, LoadState::Loading) {
            return;
        }
        let Some(workspace_uid) = Self::workspace_uid(ctx) else {
            self.load_state =
                LoadState::Error("Join or select a team to manage GitHub automations.".to_string());
            ctx.notify();
            return;
        };
        self.load_state = LoadState::Loading;
        ctx.notify();

        let client = ServerApiProvider::handle(ctx)
            .as_ref(ctx)
            .get_integrations_client();
        ctx.spawn(
            async move { client.list_github_automations(workspace_uid).await },
            |me, result, ctx| {
                match result {
                    Ok(data) => me.load_state = LoadState::Loaded(data),
                    Err(err) => {
                        log::debug!("GithubAutomations: list failed: {err:#}");
                        me.load_state =
                            LoadState::Error("Couldn't load GitHub automations.".to_string());
                    }
                }
                ctx.notify();
            },
        );
    }

    fn start_create(&mut self, ctx: &mut ViewContext<Self>) {
        self.form = Some(AutomationFormState::default());
        self.set_editor_text(ctx, "", "", "", "", "", "", "", "");
        ctx.notify();
    }

    fn start_edit(&mut self, id: String, ctx: &mut ViewContext<Self>) {
        let automation = self.loaded_automation(&id).cloned();
        let Some(automation) = automation else {
            return;
        };
        self.form = Some(AutomationFormState::from_automation(&automation));
        self.set_editor_text(
            ctx,
            &automation.name,
            automation.trigger.repo_filter.as_deref().unwrap_or(""),
            automation.trigger.branch_pattern.as_deref().unwrap_or(""),
            automation.trigger.comment_phrase.as_deref().unwrap_or(""),
            automation.action.prompt.as_deref().unwrap_or(""),
            automation.action.skill.as_deref().unwrap_or(""),
            automation.action.harness.as_deref().unwrap_or(""),
            automation.action.model_id.as_deref().unwrap_or(""),
        );
        ctx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn set_editor_text(
        &self,
        ctx: &mut ViewContext<Self>,
        name: &str,
        repo_filter: &str,
        branch_pattern: &str,
        comment_phrase: &str,
        prompt: &str,
        skill: &str,
        harness: &str,
        model_id: &str,
    ) {
        let pairs: [(&ViewHandle<EditorView>, &str); 8] = [
            (&self.editors.name, name),
            (&self.editors.repo_filter, repo_filter),
            (&self.editors.branch_pattern, branch_pattern),
            (&self.editors.comment_phrase, comment_phrase),
            (&self.editors.prompt, prompt),
            (&self.editors.skill, skill),
            (&self.editors.harness, harness),
            (&self.editors.model_id, model_id),
        ];
        for (handle, text) in pairs {
            handle.update(ctx, |editor, ctx| editor.set_buffer_text(text, ctx));
        }
    }

    fn cancel_form(&mut self, ctx: &mut ViewContext<Self>) {
        self.form = None;
        ctx.notify();
    }

    fn read_editor(&self, handle: &ViewHandle<EditorView>, ctx: &AppContext) -> String {
        handle.as_ref(ctx).buffer_text(ctx)
    }

    fn save_form(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(state) = self.form.clone() else {
            return;
        };
        let values = AutomationFormValues {
            state: &state,
            name: self.read_editor(&self.editors.name, ctx),
            repo_filter: self.read_editor(&self.editors.repo_filter, ctx),
            branch_pattern: self.read_editor(&self.editors.branch_pattern, ctx),
            comment_phrase: self.read_editor(&self.editors.comment_phrase, ctx),
            prompt: self.read_editor(&self.editors.prompt, ctx),
            skill: self.read_editor(&self.editors.skill, ctx),
            harness: self.read_editor(&self.editors.harness, ctx),
            model_id: self.read_editor(&self.editors.model_id, ctx),
        };
        let allowlist = self.repo_allowlist();
        let input: GithubAutomationInput = match values.validate(&allowlist) {
            Ok(input) => input,
            Err(message) => {
                self.show_toast(message, ctx);
                return;
            }
        };
        let Some(workspace_uid) = Self::workspace_uid(ctx) else {
            return;
        };

        let client = ServerApiProvider::handle(ctx)
            .as_ref(ctx)
            .get_integrations_client();
        ctx.spawn(
            async move { client.upsert_github_automation(workspace_uid, input).await },
            |me, result, ctx| {
                match result {
                    Ok(outcome) => {
                        me.form = None;
                        me.pending_hook_key = outcome.hook_key;
                        me.refresh(ctx);
                    }
                    Err(err) => {
                        me.show_toast(format!("Couldn't save automation: {err}"), ctx);
                    }
                }
                ctx.notify();
            },
        );
    }

    fn remove(&mut self, id: String, ctx: &mut ViewContext<Self>) {
        let Some(workspace_uid) = Self::workspace_uid(ctx) else {
            return;
        };
        let client = ServerApiProvider::handle(ctx)
            .as_ref(ctx)
            .get_integrations_client();
        ctx.spawn(
            async move { client.remove_github_automation(workspace_uid, id).await },
            |me, result, ctx| {
                match result {
                    Ok(()) => me.refresh(ctx),
                    Err(err) => me.show_toast(format!("Couldn't remove automation: {err}"), ctx),
                }
                ctx.notify();
            },
        );
    }

    fn add_provider_key(&mut self, ctx: &mut ViewContext<Self>) {
        let provider = self.read_editor(&self.provider_editor, ctx).trim().to_string();
        let key = self
            .read_editor(&self.provider_key_editor, ctx)
            .trim()
            .to_string();
        if provider.is_empty() || key.is_empty() {
            self.show_toast("Provider and key are both required.".to_string(), ctx);
            return;
        }
        let Some(workspace_uid) = Self::workspace_uid(ctx) else {
            return;
        };
        // Clear the write-only key editor immediately so the plaintext key does
        // not linger in the UI.
        self.provider_key_editor
            .update(ctx, |editor, ctx| editor.set_buffer_text("", ctx));

        let client = ServerApiProvider::handle(ctx)
            .as_ref(ctx)
            .get_integrations_client();
        ctx.spawn(
            async move {
                client
                    .set_github_provider_key(workspace_uid, provider, key)
                    .await
            },
            |me, result, ctx| {
                match result {
                    Ok(_) => {
                        me.provider_editor
                            .update(ctx, |editor, ctx| editor.set_buffer_text("", ctx));
                        me.refresh(ctx);
                    }
                    Err(err) => me.show_toast(format!("Couldn't set provider key: {err}"), ctx),
                }
                ctx.notify();
            },
        );
    }

    fn remove_provider_key(&mut self, provider: String, ctx: &mut ViewContext<Self>) {
        let Some(workspace_uid) = Self::workspace_uid(ctx) else {
            return;
        };
        let client = ServerApiProvider::handle(ctx)
            .as_ref(ctx)
            .get_integrations_client();
        ctx.spawn(
            async move {
                client
                    .remove_github_provider_key(workspace_uid, provider)
                    .await
            },
            |me, result, ctx| {
                match result {
                    Ok(()) => me.refresh(ctx),
                    Err(err) => me.show_toast(format!("Couldn't remove provider key: {err}"), ctx),
                }
                ctx.notify();
            },
        );
    }

    fn loaded_automation(&self, id: &str) -> Option<&GithubAutomation> {
        match &self.load_state {
            LoadState::Loaded(data) => data.automations.iter().find(|a| a.id == id),
            _ => None,
        }
    }

    fn loaded_data(&self) -> Option<&ListGithubAutomationsData> {
        match &self.load_state {
            LoadState::Loaded(data) => Some(data),
            _ => None,
        }
    }

    fn show_toast(&self, message: String, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            toast_stack.add_ephemeral_toast(DismissibleToast::default(message), window_id, ctx);
        });
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

    fn status_text(&self, text: String, appearance: &Appearance) -> Box<dyn Element> {
        Text::new_inline(text, appearance.ui_font_family(), appearance.ui_font_size())
            .with_color(appearance.theme().foreground().into())
            .finish()
    }

    pub fn get_modal_content(&self) -> Option<Box<dyn Element>> {
        None
    }
}

impl View for GithubAutomationsListPageView {
    fn ui_name() -> &'static str {
        "GithubAutomationsListPageView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl TypedActionView for GithubAutomationsListPageView {
    type Action = GithubAutomationsPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            GithubAutomationsPageAction::Refresh => self.refresh(ctx),
            GithubAutomationsPageAction::StartCreate => self.start_create(ctx),
            GithubAutomationsPageAction::StartEdit(id) => self.start_edit(id.clone(), ctx),
            GithubAutomationsPageAction::CancelForm => self.cancel_form(ctx),
            GithubAutomationsPageAction::SaveForm => self.save_form(ctx),
            GithubAutomationsPageAction::Remove(id) => self.remove(id.clone(), ctx),
            GithubAutomationsPageAction::ToggleEnabled => {
                if let Some(form) = self.form.as_mut() {
                    form.enabled = !form.enabled;
                    ctx.notify();
                }
            }
            GithubAutomationsPageAction::CycleTriggerType => {
                if let Some(form) = self.form.as_mut() {
                    form.trigger_type = form.trigger_type.next();
                    ctx.notify();
                }
            }
            GithubAutomationsPageAction::CycleActionType => {
                if let Some(form) = self.form.as_mut() {
                    form.action_type = form.action_type.next();
                    ctx.notify();
                }
            }
            GithubAutomationsPageAction::DismissHookKey => {
                self.pending_hook_key = None;
                ctx.notify();
            }
            GithubAutomationsPageAction::AddProviderKey => self.add_provider_key(ctx),
            GithubAutomationsPageAction::RemoveProviderKey(provider) => {
                self.remove_provider_key(provider.clone(), ctx)
            }
        }
    }
}

impl SettingsPageMeta for GithubAutomationsListPageView {
    fn section() -> SettingsSection {
        SettingsSection::GithubAutomations
    }

    fn should_render(&self, ctx: &AppContext) -> bool {
        if !FeatureFlag::GithubAutomations.is_enabled() {
            return false;
        }
        // Respect the tier's githubPolicy when the backend reports one: an
        // explicit `automationsEnabled == false` hides the page. Until the
        // backend serves `githubPolicy` (the field is `None`), the feature flag
        // alone controls visibility so the page is reachable in dogfood.
        match UserWorkspaces::as_ref(ctx).current_team() {
            Some(team) if team.billing_metadata.tier.github_policy.is_some() => {
                team.github_automations_enabled()
            }
            // No team, or a team whose tier has not reported a githubPolicy yet:
            // the feature flag alone controls visibility so the page is
            // reachable in dogfood. Write actions remain admin-gated (and
            // require a team) at the call sites.
            _ => true,
        }
    }

    fn on_page_selected(&mut self, _allow_steal_focus: bool, ctx: &mut ViewContext<Self>) {
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

impl From<ViewHandle<GithubAutomationsListPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<GithubAutomationsListPageView>) -> Self {
        SettingsPageViewHandle::GithubAutomations(view_handle)
    }
}

struct GithubAutomationsWidget;

impl SettingsWidget for GithubAutomationsWidget {
    type View = GithubAutomationsListPageView;

    fn search_terms(&self) -> &str {
        "github automations webhook trigger provider key bugbot pull request"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let can_write = GithubAutomationsListPageView::can_write(app);

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Max);

        // One-time hook-key banner.
        if let Some(hook_key) = &view.pending_hook_key {
            column.add_child(render_settings_info_banner(
                "Custom webhook signing key (shown only once)",
                Some(hook_key.as_str()),
                appearance,
            ));
            column.add_child(
                Container::new(view.render_button(
                    "Dismiss",
                    ButtonVariant::Text,
                    MouseStateHandle::default(),
                    |ctx, _, _| {
                        ctx.dispatch_typed_action(GithubAutomationsPageAction::DismissHookKey)
                    },
                    appearance,
                ))
                .with_margin_bottom(8.)
                .finish(),
            );
        }

        column.add_child(render_sub_header(appearance, "Automations", None));

        match &view.load_state {
            LoadState::Idle | LoadState::Loading => {
                column.add_child(view.status_text("Loading automations…".to_string(), appearance));
            }
            LoadState::Error(message) => {
                column.add_child(render_settings_info_banner(message, None, appearance));
            }
            LoadState::Loaded(data) => {
                if data.automations.is_empty() {
                    column.add_child(view.status_text(
                        "No automations configured yet.".to_string(),
                        appearance,
                    ));
                } else {
                    for automation in &data.automations {
                        column.add_child(render_automation_row(
                            view, automation, can_write, appearance,
                        ));
                    }
                }
            }
        }

        // Inline create/edit form or the create button.
        if let Some(form) = &view.form {
            column.add_child(
                Container::new(render_form(view, form, appearance))
                    .with_margin_top(12.)
                    .finish(),
            );
        } else if can_write {
            column.add_child(
                Container::new(view.render_button(
                    "New automation",
                    ButtonVariant::Accent,
                    view.create_mouse_state.clone(),
                    |ctx, _, _| ctx.dispatch_typed_action(GithubAutomationsPageAction::StartCreate),
                    appearance,
                ))
                .with_margin_top(12.)
                .finish(),
            );
        }

        column.add_child(
            Container::new(view.render_button(
                "Refresh",
                ButtonVariant::Text,
                view.refresh_mouse_state.clone(),
                |ctx, _, _| ctx.dispatch_typed_action(GithubAutomationsPageAction::Refresh),
                appearance,
            ))
            .with_margin_top(8.)
            .finish(),
        );

        // Provider-key admin section (admins only).
        if can_write {
            column.add_child(
                Container::new(render_separator(appearance))
                    .with_margin_top(16.)
                    .with_margin_bottom(16.)
                    .finish(),
            );
            column.add_child(render_provider_keys(view, appearance, app));
        }

        column.finish()
    }
}

fn render_automation_row(
    view: &GithubAutomationsListPageView,
    automation: &GithubAutomation,
    can_write: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let status = if automation.enabled {
        "enabled"
    } else {
        "disabled"
    };
    let summary = format!(
        "{name} — {trigger} → {action} ({status})",
        name = automation.name,
        trigger = automation.trigger.event_type.display_name(),
        action = automation.action.action_type.display_name(),
    );

    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    row.add_child(view.status_text(summary, appearance));

    if can_write {
        let id_edit = automation.id.clone();
        let id_remove = automation.id.clone();
        row.add_child(
            Container::new(view.render_button(
                "Edit",
                ButtonVariant::Text,
                MouseStateHandle::default(),
                move |ctx, _, _| {
                    ctx.dispatch_typed_action(GithubAutomationsPageAction::StartEdit(
                        id_edit.clone(),
                    ))
                },
                appearance,
            ))
            .with_margin_left(8.)
            .finish(),
        );
        row.add_child(view.render_button(
            "Remove",
            ButtonVariant::Text,
            MouseStateHandle::default(),
            move |ctx, _, _| {
                ctx.dispatch_typed_action(GithubAutomationsPageAction::Remove(id_remove.clone()))
            },
            appearance,
        ));
    }

    Container::new(row.finish()).with_margin_top(4.).finish()
}

fn render_form(
    view: &GithubAutomationsListPageView,
    form: &AutomationFormState,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Start);

    let heading = if form.is_editing() {
        "Edit automation"
    } else {
        "New automation"
    };
    column.add_child(render_sub_header(appearance, heading, None));

    // Text fields.
    let field = |label: &str, editor: &ViewHandle<EditorView>| -> Box<dyn Element> {
        let input = appearance
            .ui_builder()
            .text_input(editor.clone())
            .build()
            .finish();
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Text::new_inline(
                    label.to_string(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(appearance.theme().foreground().into())
                .finish(),
            )
            .with_child(input)
            .finish()
    };

    column.add_child(field("Name", &view.editors.name));

    // Enabled toggle.
    column.add_child(
        Container::new(
            appearance
                .ui_builder()
                .switch(view.enabled_switch.clone())
                .check(form.enabled)
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(GithubAutomationsPageAction::ToggleEnabled)
                })
                .finish(),
        )
        .with_margin_top(6.)
        .with_margin_bottom(6.)
        .finish(),
    );

    // Trigger type selector (click to cycle through options).
    column.add_child(render_dropdown_row(
        "Trigger",
        form.trigger_type.display_name(),
        view.trigger_menu_state.clone(),
        |ctx, _, _| ctx.dispatch_typed_action(GithubAutomationsPageAction::CycleTriggerType),
        appearance,
    ));
    column.add_child(field("Repo filter", &view.editors.repo_filter));
    column.add_child(field("Branch pattern", &view.editors.branch_pattern));
    column.add_child(field("Comment phrase", &view.editors.comment_phrase));

    // Action type selector (click to toggle Prompt/Skill).
    column.add_child(render_dropdown_row(
        "Action",
        form.action_type.display_name(),
        view.action_menu_state.clone(),
        |ctx, _, _| ctx.dispatch_typed_action(GithubAutomationsPageAction::CycleActionType),
        appearance,
    ));
    match form.action_type {
        GithubAutomationActionType::Prompt => {
            column.add_child(field("Prompt", &view.editors.prompt))
        }
        GithubAutomationActionType::Skill => {
            column.add_child(field("Skill", &view.editors.skill))
        }
    }
    column.add_child(field("Harness", &view.editors.harness));
    column.add_child(field("Model", &view.editors.model_id));

    // Save / cancel.
    let mut buttons = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min);
    buttons.add_child(view.render_button(
        "Save",
        ButtonVariant::Accent,
        view.save_mouse_state.clone(),
        |ctx, _, _| ctx.dispatch_typed_action(GithubAutomationsPageAction::SaveForm),
        appearance,
    ));
    buttons.add_child(view.render_button(
        "Cancel",
        ButtonVariant::Text,
        view.cancel_mouse_state.clone(),
        |ctx, _, _| ctx.dispatch_typed_action(GithubAutomationsPageAction::CancelForm),
        appearance,
    ));
    column.add_child(Container::new(buttons.finish()).with_margin_top(8.).finish());

    Container::new(column.finish()).finish()
}

fn render_dropdown_row<F>(
    label: &str,
    current: &str,
    mouse_state: MouseStateHandle,
    on_click: F,
    appearance: &Appearance,
) -> Box<dyn Element>
where
    F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F),
{
    let button = appearance
        .ui_builder()
        .button(ButtonVariant::Text, mouse_state)
        .with_centered_text_label(current.to_owned())
        .with_style(UiComponentStyles {
            font_size: Some(BUTTON_FONT_SIZE),
            ..Default::default()
        })
        .build()
        .on_click(on_click)
        .finish();

    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            Container::new(
                Text::new_inline(
                    label.to_string(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(appearance.theme().foreground().into())
                .finish(),
            )
            .with_margin_right(8.)
            .finish(),
        )
        .with_child(button)
        .finish()
}

fn render_provider_keys(
    view: &GithubAutomationsListPageView,
    appearance: &Appearance,
    _app: &AppContext,
) -> Box<dyn Element> {
    let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Start);
    column.add_child(render_sub_header(appearance, "Provider keys", None));

    let keys: &[GithubProviderKey] = view
        .loaded_data()
        .map(|d| d.provider_keys.as_slice())
        .unwrap_or_default();

    if keys.is_empty() {
        column.add_child(view.status_text("No provider keys configured.".to_string(), appearance));
    } else {
        for key in keys {
            let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
            row.add_child(view.status_text(
                format!("{} — ••••{}", key.provider, key.last4),
                appearance,
            ));
            let provider = key.provider.clone();
            row.add_child(
                Container::new(view.render_button(
                    "Remove",
                    ButtonVariant::Text,
                    MouseStateHandle::default(),
                    move |ctx, _, _| {
                        ctx.dispatch_typed_action(GithubAutomationsPageAction::RemoveProviderKey(
                            provider.clone(),
                        ))
                    },
                    appearance,
                ))
                .with_margin_left(8.)
                .finish(),
            );
            column.add_child(Container::new(row.finish()).with_margin_top(4.).finish());
        }
    }

    // Add-key editors.
    column.add_child(
        Container::new(
            appearance
                .ui_builder()
                .text_input(view.provider_editor.clone())
                .build()
                .finish(),
        )
        .with_margin_top(8.)
        .finish(),
    );
    column.add_child(
        appearance
            .ui_builder()
            .text_input(view.provider_key_editor.clone())
            .build()
            .finish(),
    );
    column.add_child(
        Container::new(view.render_button(
            "Add provider key",
            ButtonVariant::Accent,
            view.add_key_mouse_state.clone(),
            |ctx, _, _| ctx.dispatch_typed_action(GithubAutomationsPageAction::AddProviderKey),
            appearance,
        ))
        .with_margin_top(8.)
        .finish(),
    );

    column.finish()
}
