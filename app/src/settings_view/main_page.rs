use std::sync::{Arc, Mutex};

use ::settings::{Setting, ToggleableSetting};
use lazy_static::lazy_static;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::channel::ChannelState;
use warp_core::context_flag::ContextFlag;
use warp_core::features::FeatureFlag;
use warp_core::ui::icons::Icon;
#[cfg(not(target_family = "wasm"))]
use warp_server_client::iap::{IapCredentialsState, IapManager, IapManagerEvent};
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{
    Align, Border, CacheOption, ChildView, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Dismiss, Element, Empty, Flex, Hoverable, Image, MainAxisAlignment,
    MainAxisSize, MouseStateHandle, ParentElement, Radius, Shrinkable, Text,
};
use warpui::fonts::Weight;
use warpui::keymap::ContextPredicate;
use warpui::platform::Cursor;
use warpui::ui_components::button::{ButtonVariant, TextAndIcon, TextAndIconAlignment};
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{
    id, Action, AppContext, Entity, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use super::settings_page::{
    render_body_item, render_customer_type_badge, AdditionalInfo, LocalOnlyIconState, MatchData,
    PageType, SettingsPageMeta, SettingsPageViewHandle, SettingsWidget, ToggleState,
    HEADER_PADDING,
};
use super::{flags, SettingsAction, SettingsSection, ToggleSettingActionPair};
use crate::appearance::Appearance;
use crate::auth::auth_manager::{AuthManager, LoginGatedFeature};
use crate::auth::auth_state::AuthState;
use crate::auth::auth_view_modal::AuthViewVariant;
use crate::auth::{AuthStateProvider, UserUid};
use crate::autoupdate::{self, AutoupdateStage, AutoupdateState};
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
    TextOptions,
};
use crate::server::ids::ServerId;
use crate::settings::cloud_preferences::CloudPreferencesSettings;
use crate::workspace::WorkspaceAction;
use crate::workspaces::update_manager::TeamUpdateManager;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::CustomerType;
use crate::{report_if_error, send_telemetry_from_ctx, TelemetryEvent};

const PHOTO_SIZE: f32 = 40.;
/// Fixed width for the inline display-name editor so the row doesn't reflow as
/// the typed text grows/shrinks.
const NAME_EDITOR_WIDTH: f32 = 240.;
const REFERRAL_CTA: &str = "Earn rewards by sharing Bang with friends & colleagues";
const REGULAR_TEXT_FONT_SIZE: f32 = 12.;
const VERTICAL_MARGIN: f32 = 24.;
const LOG_OUT_TEXT: &str = "Log out";
lazy_static! {
    static ref SETTINGS_SYNC_BINDINGS_ADDED: Arc<Mutex<bool>> = Default::default();
}

pub fn init_actions_from_parent_view<T: Action + Clone>(
    app: &mut AppContext,
    context: &ContextPredicate,
    builder: fn(SettingsAction) -> T,
) {
    let mut toggle_binding_pairs = Vec::new();
    maybe_add_settings_sync_toggle_binding(app, context, builder, &mut toggle_binding_pairs);

    // Add other bindings here in the future.

    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(toggle_binding_pairs, app);
}

fn maybe_add_settings_sync_toggle_binding<T: Action + Clone>(
    app: &mut AppContext,
    context: &ContextPredicate,
    builder: fn(SettingsAction) -> T,
    toggle_binding_pairs: &mut Vec<ToggleSettingActionPair<T>>,
) {
    let mut lock = SETTINGS_SYNC_BINDINGS_ADDED
        .lock()
        .expect("settings sync bindings lock poisoned");
    if !*lock {
        *lock = true;
        toggle_binding_pairs.push(
            ToggleSettingActionPair::new(
                "settings sync",
                builder(SettingsAction::MainPageToggle(
                    MainPageAction::ToggleSettingsSync,
                )),
                context,
                flags::SETTINGS_SYNC_FLAG,
            )
            .is_supported_on_current_platform(
                CloudPreferencesSettings::as_ref(app)
                    .settings_sync_enabled
                    .is_supported_on_current_platform(),
            ),
        );
    }
}

pub fn handle_experiment_change(app: &mut AppContext) {
    let mut toggle_binding_pairs: Vec<ToggleSettingActionPair<WorkspaceAction>> = Vec::new();
    maybe_add_settings_sync_toggle_binding(
        app,
        &id!("Workspace"),
        WorkspaceAction::DispatchToSettingsTab,
        &mut toggle_binding_pairs,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(toggle_binding_pairs, app);
}

#[derive(Debug, Clone)]
pub enum MainPageAction {
    Relaunch,
    DownloadUpdate,
    CheckForUpdate,
    ToggleSettingsSync,
    Upgrade {
        team_uid: Option<ServerId>,
        user_id: UserUid,
    },
    GenerateStripeBillingPortalLink {
        team_uid: ServerId,
    },
    SignupAnonymousUser,
    OpenUrl(String),
    /// Enter inline edit mode for the account display name.
    BeginEditName,
    /// Commit the in-progress display-name edit (fired when the user clicks
    /// outside the inline editor).
    CommitNameEdit,
    #[cfg(not(target_family = "wasm"))]
    RefreshIapCredentials,
}

impl MainPageAction {
    fn blocked_for_anonymous_user(&self) -> bool {
        use MainPageAction::*;
        matches!(
            self,
            Upgrade { .. } | GenerateStripeBillingPortalLink { .. } | ToggleSettingsSync,
        )
    }
}

impl From<&MainPageAction> for LoginGatedFeature {
    fn from(val: &MainPageAction) -> LoginGatedFeature {
        use MainPageAction::*;
        match val {
            Upgrade { .. } => "Upgrade Plan",
            GenerateStripeBillingPortalLink { .. } => "Generate Stripe Billing Portal Link",
            ToggleSettingsSync => "Toggle Settings Sync",
            _ => "Unknown reason",
        }
    }
}

#[derive(Clone, Copy)]
pub enum MainSettingsPageEvent {
    CheckForUpdate,
    #[allow(dead_code)]
    OpenWarpDrive,
    SignupAnonymousUser,
}

pub struct MainSettingsPageView {
    page: PageType<Self>,
    auth_state: Arc<AuthState>,
    /// Inline editor for the account display name. Rendered in place of the name
    /// text while `is_editing_name` is true; commits on Enter or blur.
    name_editor: ViewHandle<EditorView>,
    /// Whether the display-name field is currently in edit mode.
    is_editing_name: bool,
}

impl Entity for MainSettingsPageView {
    type Event = MainSettingsPageEvent;
}

impl TypedActionView for MainSettingsPageView {
    type Action = MainPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        // Block anonymous users from upgrading
        if AuthStateProvider::as_ref(ctx)
            .get()
            .is_anonymous_or_logged_out()
            && action.blocked_for_anonymous_user()
        {
            AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                auth_manager.attempt_login_gated_feature(
                    action.into(),
                    AuthViewVariant::RequireLoginCloseable,
                    ctx,
                )
            });
            return;
        }

        match action {
            MainPageAction::Relaunch => {
                autoupdate::initiate_relaunch_for_update(ctx);
            }
            MainPageAction::DownloadUpdate => {
                autoupdate::manually_download_new_version(ctx);
            }
            MainPageAction::CheckForUpdate => {
                ctx.emit(MainSettingsPageEvent::CheckForUpdate);
                ctx.notify();
            }
            MainPageAction::ToggleSettingsSync => {
                let new_value =
                    CloudPreferencesSettings::handle(ctx).update(ctx, |prefs_settings, ctx| {
                        report_if_error!(prefs_settings
                            .settings_sync_enabled
                            .toggle_and_save_value(ctx));
                        *prefs_settings.settings_sync_enabled
                    });
                send_telemetry_from_ctx!(
                    TelemetryEvent::ToggleSettingsSync {
                        is_settings_sync_enabled: new_value,
                    },
                    ctx
                );
                ctx.notify();
            }
            MainPageAction::Upgrade { team_uid, user_id } => match team_uid {
                Some(team_uid) => {
                    ctx.open_url(&UserWorkspaces::upgrade_link_for_team(*team_uid));
                }
                None => {
                    ctx.open_url(&UserWorkspaces::upgrade_link(*user_id));
                }
            },
            MainPageAction::GenerateStripeBillingPortalLink { team_uid } => {
                UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                    user_workspaces.generate_stripe_billing_portal_link(*team_uid, ctx);
                });
            }
            MainPageAction::SignupAnonymousUser => {
                ctx.emit(MainSettingsPageEvent::SignupAnonymousUser);
            }
            MainPageAction::OpenUrl(url) => {
                ctx.open_url(url);
            }
            MainPageAction::BeginEditName => {
                self.begin_editing_name(ctx);
            }
            MainPageAction::CommitNameEdit => {
                self.commit_name_edit(ctx);
            }
            #[cfg(not(target_family = "wasm"))]
            MainPageAction::RefreshIapCredentials => {
                IapManager::handle(ctx).update(ctx, |manager, ctx| manager.start_refresh(ctx));
                ctx.notify();
            }
        }
    }
}

impl View for MainSettingsPageView {
    fn ui_name() -> &'static str {
        "MainSettingsPage"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl MainSettingsPageView {
    pub fn new(ctx: &mut ViewContext<MainSettingsPageView>) -> Self {
        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();

        let autoupdate_state_handle = AutoupdateState::handle(ctx);
        ctx.observe(
            &autoupdate_state_handle,
            Self::handle_autoupdate_state_change,
        );

        ctx.subscribe_to_model(&CloudPreferencesSettings::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });

        let auth_manager_handle = AuthManager::handle(ctx);
        ctx.subscribe_to_model(&auth_manager_handle, |_, _, _, ctx| {
            ctx.notify();
        });

        let mut widgets: Vec<Box<dyn SettingsWidget<View = Self>>> = vec![
            Box::new(AccountWidget::default()),
            Box::new(DividerWidget {}),
        ];

        widgets.push(Box::new(SettingsSyncWidget::default()));

        widgets.push(Box::new(EarnRewardsWidget::default()));

        #[cfg(not(target_family = "wasm"))]
        if IapManager::as_ref(ctx).is_enabled() {
            widgets.push(Box::new(IapCredentialsWidget::default()));
            let iap_manager_handle = IapManager::handle(ctx);
            ctx.subscribe_to_model(&iap_manager_handle, |_, _, e, ctx| {
                if matches!(e, IapManagerEvent::StateChanged) {
                    ctx.notify();
                }
            })
        }

        if ChannelState::app_version().is_some() {
            widgets.push(Box::new(VersionInfoWidget::default()));
        }

        widgets.push(Box::new(LogoutWidget::default()));

        let page = PageType::new_uncategorized(widgets, Some("Account"));

        let name_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let options = SingleLineEditorOptions {
                // Match the display-name label sizing so the inline editor doesn't
                // reflow the row when edit mode toggles.
                text: TextOptions::ui_text(Some(16.), appearance),
                // Mirror the proven tab-rename editor: select the current name on
                // focus and drop the selection on blur.
                select_all_on_focus: true,
                clear_selections_on_blur: true,
                // Up/down shouldn't page through settings while editing the name.
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text("Display name", ctx);
            editor
        });
        ctx.subscribe_to_view(&name_editor, Self::handle_name_editor_event);

        MainSettingsPageView {
            page,
            auth_state,
            name_editor,
            is_editing_name: false,
        }
    }

    /// Enters display-name edit mode: pre-fills the editor with the current name,
    /// selects it, and focuses it so the user can type immediately.
    fn begin_editing_name(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_editing_name {
            return;
        }
        let current = self.auth_state.display_name().unwrap_or_default();
        self.is_editing_name = true;
        self.name_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(&current, ctx);
            editor.select_all(ctx);
        });
        ctx.focus(&self.name_editor);
        ctx.notify();
    }

    /// Commits the in-progress name edit (from Enter or blur), pushing the new
    /// value through `AuthManager` (local state + backend sync) and leaving edit
    /// mode. Ignores the commit if edit mode is already closed.
    fn commit_name_edit(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.is_editing_name {
            return;
        }
        self.is_editing_name = false;
        let new_name = self
            .name_editor
            .as_ref(ctx)
            .buffer_text(ctx)
            .trim()
            .to_owned();
        if !new_name.is_empty() && Some(&new_name) != self.auth_state.display_name().as_ref() {
            AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                auth_manager.update_display_name(new_name, ctx);
            });
        }
        ctx.notify();
    }

    fn handle_name_editor_event(
        &mut self,
        _: ViewHandle<EditorView>,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            // Enter blurs the editor (below), so committing here and on Blurred is
            // guarded by `is_editing_name` to avoid a double commit.
            EditorEvent::Enter => {
                self.commit_name_edit(ctx);
                ctx.focus_self();
            }
            EditorEvent::Blurred => self.commit_name_edit(ctx),
            _ => {}
        }
    }

    fn handle_autoupdate_state_change(
        &mut self,
        _: ModelHandle<AutoupdateState>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.notify();
    }
}

#[derive(Default)]
struct AccountWidgetStateHandles {
    upgrade_link: MouseStateHandle,
    anonymous_user_sign_up_button: MouseStateHandle,
    enterprise_contact_us_link: MouseStateHandle,
    stripe_billing_portal_link: MouseStateHandle,
    name_edit: MouseStateHandle,
}

#[derive(Default)]
struct AccountWidget {
    ui_state_handles: AccountWidgetStateHandles,
}

impl AccountWidget {
    fn render_anonymous_account_info(
        &self,
        auth_state: &AuthState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let button_styles = UiComponentStyles {
            font_size: Some(14.),
            font_weight: Some(Weight::Semibold),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
            padding: Some(Coords {
                top: 12.,
                bottom: 12.,
                left: 40.,
                right: 40.,
            }),
            ..Default::default()
        };

        let user_info = appearance
            .ui_builder()
            .button(
                ButtonVariant::Accent,
                self.ui_state_handles.anonymous_user_sign_up_button.clone(),
            )
            .with_style(button_styles)
            .with_text_label("Sign up".to_owned())
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(MainPageAction::SignupAnonymousUser);
            })
            .finish();

        let mut plan_info = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::SpaceEvenly)
            .with_cross_axis_alignment(CrossAxisAlignment::End);
        let current_user_id = auth_state.user_id().unwrap_or_default();

        plan_info.add_child(render_customer_type_badge(appearance, "Free".into()));
        plan_info.add_child(
            Container::new(
                appearance
                    .ui_builder()
                    .button(
                        ButtonVariant::Link,
                        self.ui_state_handles.upgrade_link.clone(),
                    )
                    .with_text_and_icon_label(
                        TextAndIcon::new(
                            TextAndIconAlignment::IconFirst,
                            "Compare plans",
                            Icon::CoinsStacked.to_warpui_icon(appearance.theme().accent()),
                            MainAxisSize::Min,
                            MainAxisAlignment::Center,
                            vec2f(14., 14.),
                        )
                        .with_inner_padding(4.),
                    )
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(MainPageAction::Upgrade {
                            team_uid: None,
                            user_id: current_user_id,
                        });
                    })
                    .finish(),
            )
            .with_margin_top(8.)
            .finish(),
        );

        Flex::row()
            .with_child(
                Shrinkable::new(
                    1.0,
                    Flex::row()
                        .with_child(user_info)
                        .with_main_axis_alignment(MainAxisAlignment::Start)
                        .with_main_axis_size(MainAxisSize::Max)
                        .finish(),
                )
                .finish(),
            )
            .with_child(Align::new(plan_info.finish()).right().finish())
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .finish()
    }

    /// Renders the display-name line: an inline single-line editor while the view
    /// is in name-edit mode, otherwise a click-to-edit text label. Clicking the
    /// label enters edit mode (see `MainPageAction::BeginEditName`).
    fn render_display_name_line(
        &self,
        view: &MainSettingsPageView,
        screen_name: &str,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        if view.is_editing_name {
            // Clicking a non-focusable part of the settings page won't blur the
            // editor, so `Dismiss` catches clicks outside the editor and commits —
            // matching the click-outside-to-commit behavior of the other inline
            // editors on the settings pages. (Enter/blur still commit via the
            // editor event subscription.)
            return Dismiss::new(
                ConstrainedBox::new(ChildView::new(&view.name_editor).finish())
                    .with_width(NAME_EDITOR_WIDTH)
                    .finish(),
            )
            .on_dismiss(|ctx, _| {
                ctx.dispatch_typed_action(MainPageAction::CommitNameEdit);
            })
            .finish();
        }

        let name = screen_name.to_owned();
        Hoverable::new(self.ui_state_handles.name_edit.clone(), move |_| {
            Text::new_inline(name.clone(), appearance.ui_font_family(), 16.)
                .with_color(appearance.theme().active_ui_text_color().into())
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(MainPageAction::BeginEditName);
        })
        .finish()
    }

    fn render_account_info(
        &self,
        view: &MainSettingsPageView,
        profile_image_source: Option<&AssetSource>,
        auth_state: &AuthState,
        app: &AppContext,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut user_info = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        if let Some(profile_image_source) = profile_image_source {
            // Only continue if profile_image_source is a source with a non empty url/path
            if matches!(profile_image_source, AssetSource::Async { ref id, .. } if !id.key().is_empty())
                || matches!(profile_image_source, AssetSource::Bundled { path, .. } if !path.is_empty())
                || matches!(profile_image_source, AssetSource::LocalFile { path, .. } if !path.is_empty())
            {
                let photo = Image::new(profile_image_source.clone(), CacheOption::BySize)
                    .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)));
                user_info.add_child(
                    Container::new(
                        ConstrainedBox::new(photo.finish())
                            .with_height(PHOTO_SIZE)
                            .with_width(PHOTO_SIZE)
                            .finish(),
                    )
                    .with_margin_right(HEADER_PADDING)
                    .finish(),
                );
            }
        }

        let display_name = auth_state.username_for_display().map(|screen_name| {
            let email = auth_state.user_email();
            match email {
                Some(email) => {
                    if !screen_name.is_empty() && screen_name != email {
                        Flex::column()
                            .with_main_axis_alignment(MainAxisAlignment::SpaceEvenly)
                            .with_cross_axis_alignment(CrossAxisAlignment::Start)
                            .with_child(self.render_display_name_line(
                                view,
                                &screen_name,
                                appearance,
                            ))
                            .with_child(
                                appearance
                                    .ui_builder()
                                    .paragraph(email)
                                    .with_style(UiComponentStyles {
                                        font_color: Some(
                                            appearance
                                                .theme()
                                                .active_ui_text_color()
                                                .with_opacity(60)
                                                .into(),
                                        ),
                                        font_size: Some(REGULAR_TEXT_FONT_SIZE),
                                        ..Default::default()
                                    })
                                    .build()
                                    .finish(),
                            )
                            .finish()
                    } else {
                        self.render_display_name_line(view, &email, appearance)
                    }
                }
                _ => self.render_display_name_line(view, &screen_name, appearance),
            }
        });

        if let Some(display_name) = display_name {
            user_info.add_child(display_name);
        }

        let mut plan_info = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::SpaceEvenly)
            .with_cross_axis_alignment(CrossAxisAlignment::End);
        let current_user_id = auth_state.user_id().unwrap_or_default();
        let workspaces = UserWorkspaces::as_ref(app);
        if let Some(team) = workspaces.current_team() {
            if team.billing_metadata.customer_type != CustomerType::Unknown {
                plan_info.add_child(render_customer_type_badge(
                    appearance,
                    team.billing_metadata.customer_type.to_display_string(),
                ));
            }

            let current_user_email = auth_state.user_email().unwrap_or_default();
            let has_admin_permissions = team.has_admin_permissions(&current_user_email);
            if has_admin_permissions {
                if team.billing_metadata.customer_type == CustomerType::Enterprise {
                    plan_info.add_child(
                        appearance
                            .ui_builder()
                            .link(
                                "Contact support".into(),
                                Some("mailto:support@warp.dev".into()),
                                None,
                                self.ui_state_handles.enterprise_contact_us_link.clone(),
                            )
                            .soft_wrap(false)
                            .build()
                            .with_margin_top(8.)
                            .finish(),
                    );
                } else {
                    if team.has_billing_history {
                        let team_uid = team.uid;
                        plan_info.add_child(
                            appearance
                                .ui_builder()
                                .link(
                                    "Manage billing".into(),
                                    None,
                                    Some(Box::new(move |ctx| {
                                        ctx.dispatch_typed_action(
                                            MainPageAction::GenerateStripeBillingPortalLink {
                                                team_uid,
                                            },
                                        );
                                    })),
                                    self.ui_state_handles.stripe_billing_portal_link.clone(),
                                )
                                .soft_wrap(false)
                                .build()
                                .with_margin_top(8.)
                                .finish(),
                        );
                    }

                    // If the team is upgradeable to self-serve tier, show them the upgrade link.
                    if team.billing_metadata.can_upgrade_to_higher_tier_plan() {
                        let description = match team.billing_metadata.customer_type {
                            CustomerType::Prosumer => "Upgrade to Turbo plan",
                            CustomerType::Turbo => "Upgrade to Lightspeed plan",
                            _ => "Compare plans",
                        };
                        let team_uid = team.uid;
                        plan_info.add_child(
                            appearance
                                .ui_builder()
                                .link(
                                    description.into(),
                                    None,
                                    Some(Box::new(move |ctx| {
                                        ctx.dispatch_typed_action(MainPageAction::Upgrade {
                                            team_uid: Some(team_uid),
                                            user_id: current_user_id,
                                        });
                                    })),
                                    self.ui_state_handles.upgrade_link.clone(),
                                )
                                .soft_wrap(false)
                                .build()
                                .with_margin_top(8.)
                                .finish(),
                        );
                    }
                }
            }
        } else {
            let plan_badge_child = render_customer_type_badge(appearance, "Free".into());
            plan_info.add_child(plan_badge_child);

            plan_info.add_child(
                appearance
                    .ui_builder()
                    .link(
                        "Compare plans".into(),
                        None,
                        Some(Box::new(move |ctx| {
                            ctx.dispatch_typed_action(MainPageAction::Upgrade {
                                team_uid: None,
                                user_id: current_user_id,
                            });
                        })),
                        self.ui_state_handles.upgrade_link.clone(),
                    )
                    .soft_wrap(false)
                    .build()
                    .with_margin_top(8.)
                    .finish(),
            );
        }

        let mut row = Flex::row()
            .with_child(
                Shrinkable::new(1.0, Align::new(user_info.finish()).left().finish()).finish(),
            )
            .with_cross_axis_alignment(CrossAxisAlignment::Start);

        if !FeatureFlag::UsageBasedPricing.is_enabled() {
            row.add_child(Align::new(plan_info.finish()).right().finish());
        }

        row.finish()
    }
}

impl SettingsWidget for AccountWidget {
    type View = MainSettingsPageView;

    fn search_terms(&self) -> &str {
        "account sign up"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let account_info = if view.auth_state.is_anonymous_or_logged_out() {
            self.render_anonymous_account_info(view.auth_state.as_ref(), appearance)
        } else {
            let profile_image_source = view.auth_state.user_photo_url().map(|url| {
                asset_cache::url_source_with_persistence(url, &warp_core::paths::cache_dir())
            });
            self.render_account_info(
                view,
                profile_image_source.as_ref(),
                view.auth_state.as_ref(),
                app,
                appearance,
            )
        };

        Flex::column()
            .with_child(
                Container::new(account_info)
                    .with_margin_top(VERTICAL_MARGIN)
                    .finish(),
            )
            .finish()
    }
}

struct DividerWidget {}

impl SettingsWidget for DividerWidget {
    type View = MainSettingsPageView;

    fn search_terms(&self) -> &str {
        ""
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        Container::new(
            Container::new(Empty::new().finish())
                .with_border(Border::bottom(1.).with_border_fill(appearance.theme().outline()))
                .finish(),
        )
        .with_margin_top(VERTICAL_MARGIN)
        .finish()
    }
}

#[derive(Default)]
struct SettingsSyncWidget {
    tooltip_state: MouseStateHandle,
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for SettingsSyncWidget {
    type View = MainSettingsPageView;

    fn search_terms(&self) -> &str {
        "settings sync"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        !AuthStateProvider::as_ref(app)
            .get()
            .is_anonymous_or_logged_out()
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let preferences_settings = CloudPreferencesSettings::as_ref(app);

        let label_info = AdditionalInfo {
            mouse_state: self.tooltip_state.clone(),
            on_click_action: Some(MainPageAction::OpenUrl(
                "https://docs.warp.dev/terminal/more-features/settings-sync".into(),
            )),
            secondary_text: None,
            tooltip_override_text: None,
        };

        Container::new(render_body_item::<MainPageAction>(
            "Settings sync".to_string(),
            Some(label_info),
            // Cloud prefs are always synced, so no need to show the local-only icon.
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*preferences_settings.settings_sync_enabled.value())
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(MainPageAction::ToggleSettingsSync)
                })
                .finish(),
            None,
        ))
        .with_margin_top(VERTICAL_MARGIN)
        .finish()
    }
}

#[derive(Default)]
struct EarnRewardsWidget {
    refer_link_mouse_handle: MouseStateHandle,
}

impl EarnRewardsWidget {
    fn render_row(
        &self,
        appearance: &Appearance,
        label: &str,
        right_child: Box<dyn Element>,
    ) -> Box<dyn Element> {
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Shrinkable::new(
                    1.0,
                    Align::new(
                        Text::new_inline(
                            label.to_string(),
                            appearance.ui_font_family(),
                            REGULAR_TEXT_FONT_SIZE,
                        )
                        .with_color(appearance.theme().active_ui_text_color().into())
                        .finish(),
                    )
                    .left()
                    .finish(),
                )
                .finish(),
            )
            .with_child(right_child)
            .finish()
    }
}

impl SettingsWidget for EarnRewardsWidget {
    type View = MainSettingsPageView;

    fn search_terms(&self) -> &str {
        "earn rewards referral share friends"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        !AuthStateProvider::as_ref(app)
            .get()
            .is_anonymous_or_logged_out()
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        Container::new(
            self.render_row(
                appearance,
                REFERRAL_CTA,
                appearance
                    .ui_builder()
                    .link(
                        "Refer a friend".into(),
                        None,
                        Some(Box::new(move |ctx| {
                            ctx.dispatch_typed_action(WorkspaceAction::ShowReferralSettingsPage);
                        })),
                        self.refer_link_mouse_handle.clone(),
                    )
                    .soft_wrap(false)
                    .build()
                    .finish(),
            ),
        )
        .with_margin_top(VERTICAL_MARGIN)
        .finish()
    }
}

#[derive(Default)]
struct VersionInfoWidget {
    copy_version_button_mouse_state: MouseStateHandle,
    version_info_cta_link_mouse_state: MouseStateHandle,
}

impl VersionInfoWidget {
    fn render_version_info(
        &self,
        version: &'static str,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let faded_text_color = appearance
            .theme()
            .active_ui_text_color()
            .with_opacity(60)
            .into();
        struct StatusContent {
            text: &'static str,
            color: ColorU,
        }
        struct CallToActionContent {
            text: &'static str,
            action: MainPageAction,
        }

        let (status_content, call_to_action_content) =
            if ContextFlag::PromptForVersionUpdates.is_enabled() {
                let ansi_red: ColorU = appearance.theme().terminal_colors().bright.red.into();
                match autoupdate::get_update_state(app) {
                    AutoupdateStage::NoUpdateAvailable => (
                        Some(StatusContent {
                            text: "Up to date",
                            color: faded_text_color,
                        }),
                        Some(CallToActionContent {
                            text: "Check for updates",
                            action: MainPageAction::CheckForUpdate,
                        }),
                    ),
                    AutoupdateStage::CheckingForUpdate => (
                        Some(StatusContent {
                            text: "checking for update...",
                            color: faded_text_color,
                        }),
                        None,
                    ),
                    AutoupdateStage::DownloadingUpdate => (
                        Some(StatusContent {
                            text: "downloading update...",
                            color: faded_text_color,
                        }),
                        None,
                    ),
                    AutoupdateStage::UpdateReady { .. } => (
                        Some(StatusContent {
                            text: "Update available",
                            color: ansi_red,
                        }),
                        Some(CallToActionContent {
                            text: "Relaunch Bang",
                            action: MainPageAction::Relaunch,
                        }),
                    ),
                    AutoupdateStage::Updating { .. } => (
                        Some(StatusContent {
                            text: "Updating...",
                            color: faded_text_color,
                        }),
                        None,
                    ),
                    AutoupdateStage::UpdatedPendingRestart { .. } => (
                        Some(StatusContent {
                            text: "Installed update",
                            color: faded_text_color,
                        }),
                        Some(CallToActionContent {
                            text: "Relaunch Bang",
                            action: MainPageAction::Relaunch,
                        }),
                    ),
                    AutoupdateStage::UnableToUpdateToNewVersion { .. } => (
                        Some(StatusContent {
                            text: "A new version of Bang is available but can't be installed",
                            color: ansi_red,
                        }),
                        Some(CallToActionContent {
                            text: "Update Bang manually",
                            // note: the handler for this action is a no-op
                            action: MainPageAction::DownloadUpdate,
                        }),
                    ),
                    AutoupdateStage::UnableToLaunchNewVersion { .. } => (
                        Some(StatusContent {
                            text: "A new version of Bang is installed but can't be launched.",
                            color: ansi_red,
                        }),
                        Some(CallToActionContent {
                            text: "Update Bang manually",
                            // note: the handler for this action is a no-op
                            action: MainPageAction::DownloadUpdate,
                        }),
                    ),
                }
            } else {
                (None, None)
            };

        let mut first_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Shrinkable::new(
                    1.0,
                    Align::new(
                        Text::new_inline(
                            "Version".to_string(),
                            appearance.ui_font_family(),
                            REGULAR_TEXT_FONT_SIZE,
                        )
                        .with_color(faded_text_color)
                        .finish(),
                    )
                    .left()
                    .finish(),
                )
                .finish(),
            );
        if let Some(call_to_action_content) = call_to_action_content {
            first_row.add_child(
                appearance
                    .ui_builder()
                    .link(
                        call_to_action_content.text.into(),
                        None,
                        Some(Box::new(move |ctx| {
                            ctx.dispatch_typed_action(call_to_action_content.action.clone());
                        })),
                        self.version_info_cta_link_mouse_state.clone(),
                    )
                    .soft_wrap(false)
                    .build()
                    .finish(),
            );
        }

        let mut second_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Shrinkable::new(
                    1.0,
                    Align::new(
                        Flex::row()
                            .with_cross_axis_alignment(CrossAxisAlignment::Start)
                            .with_child(
                                appearance
                                    .ui_builder()
                                    .copy_button(16., self.copy_version_button_mouse_state.clone())
                                    .build()
                                    .with_cursor(Cursor::PointingHand)
                                    .on_click(move |ctx, _, _| {
                                        ctx.dispatch_typed_action(WorkspaceAction::CopyVersion(
                                            version,
                                        ));
                                    })
                                    .finish(),
                            )
                            .with_child(
                                Container::new(
                                    Text::new_inline(
                                        version.to_string(),
                                        appearance.ui_font_family(),
                                        REGULAR_TEXT_FONT_SIZE,
                                    )
                                    .with_color(appearance.theme().active_ui_text_color().into())
                                    .finish(),
                                )
                                .with_margin_left(8.)
                                .finish(),
                            )
                            .finish(),
                    )
                    .left()
                    .finish(),
                )
                .finish(),
            );
        if let Some(status_content) = status_content {
            second_row.add_child(
                Text::new_inline(
                    status_content.text.to_string(),
                    appearance.ui_font_family(),
                    REGULAR_TEXT_FONT_SIZE,
                )
                .with_color(status_content.color)
                .finish(),
            );
        }

        let mut version_info = Flex::column();
        version_info.add_child(first_row.finish());
        version_info.add_child(
            Container::new(second_row.finish())
                .with_margin_top(5.)
                .finish(),
        );
        version_info.finish()
    }
}

impl SettingsWidget for VersionInfoWidget {
    type View = MainSettingsPageView;

    fn search_terms(&self) -> &str {
        "version update"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        if let Some(version) = ChannelState::app_version() {
            Container::new(self.render_version_info(version, appearance, app))
                .with_margin_top(VERTICAL_MARGIN)
                .finish()
        } else {
            log::error!("Shouldn't render VersionInfoWidget without GIT_RELEASE_TAG");
            Empty::new().finish()
        }
    }
}

#[derive(Default)]
struct LogoutWidget {
    mouse_state: MouseStateHandle,
}

impl LogoutWidget {
    fn render_logout_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .button(ButtonVariant::Secondary, self.mouse_state.clone())
            .with_text_label(LOG_OUT_TEXT.into())
            .with_style(UiComponentStyles {
                font_size: Some(14.),
                padding: Some(Coords::uniform(8.).left(32.).right(32.)),
                ..Default::default()
            })
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(WorkspaceAction::LogOut);
            })
            .finish()
    }
}

/// Widget displaying IAP credential state and a refresh button. Only
/// visible on staging channels where IAP is active.
#[cfg(not(target_family = "wasm"))]
#[derive(Default)]
struct IapCredentialsWidget {
    refresh_button_mouse_state: MouseStateHandle,
}

#[cfg(not(target_family = "wasm"))]
impl SettingsWidget for IapCredentialsWidget {
    type View = MainSettingsPageView;

    fn search_terms(&self) -> &str {
        "iap staging gcloud proxy credentials"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        // `is_enabled()` gates widget registration in `MainSettingsPageView::new`,
        // so `state()` should be `Some` here; bail out defensively though.
        let Some(state) = IapManager::as_ref(app).state() else {
            return Empty::new().finish();
        };
        let ansi_red: ColorU = appearance.theme().terminal_colors().bright.red.into();
        let disabled: ColorU = appearance.theme().disabled_ui_text_color().into();
        let active: ColorU = appearance.theme().active_ui_text_color().into();
        let (status_text, status_color): (String, ColorU) = match &state {
            IapCredentialsState::Missing => ("Not yet loaded".to_string(), disabled),
            IapCredentialsState::Refreshing { .. } => ("Refreshing…".to_string(), active),
            IapCredentialsState::Loaded(cached) => {
                let remaining = cached
                    .expires_at
                    .saturating_duration_since(instant::Instant::now());
                let mins = remaining.as_secs() / 60;
                (format!("Loaded (refreshes in ~{mins}m)"), active)
            }
            IapCredentialsState::Failed { message, .. } => (format!("Failed: {message}"), ansi_red),
            IapCredentialsState::EnvInjected { .. } => {
                ("Using injected token (WARP_IAP_TOKEN)".to_string(), active)
            }
        };

        let is_refreshing = matches!(state, IapCredentialsState::Refreshing { .. });

        let label = Align::new(
            Text::new_inline(
                "Staging IAP credentials".to_string(),
                appearance.ui_font_family(),
                REGULAR_TEXT_FONT_SIZE,
            )
            .with_color(appearance.theme().active_ui_text_color().into())
            .finish(),
        )
        .left()
        .finish();

        let status = Container::new(
            appearance
                .ui_builder()
                .paragraph(status_text)
                .with_style(UiComponentStyles {
                    font_color: Some(status_color),
                    font_size: Some(REGULAR_TEXT_FONT_SIZE),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_margin_top(4.)
        .finish();

        let refresh_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Secondary,
                self.refresh_button_mouse_state.clone(),
            )
            .with_text_label(if is_refreshing {
                "Refreshing…".into()
            } else {
                "Refresh".into()
            })
            .with_style(UiComponentStyles {
                font_size: Some(12.),
                padding: Some(Coords::uniform(6.).left(16.).right(16.)),
                ..Default::default()
            })
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(MainPageAction::RefreshIapCredentials);
            })
            .finish();

        let button_row = Container::new(Align::new(refresh_button).left().finish())
            .with_margin_top(8.)
            .finish();

        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(label)
                .with_child(status)
                .with_child(button_row)
                .finish(),
        )
        .with_margin_top(VERTICAL_MARGIN)
        .finish()
    }
}

impl SettingsWidget for LogoutWidget {
    type View = MainSettingsPageView;

    fn search_terms(&self) -> &str {
        "sign out log out logout"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        !AuthStateProvider::as_ref(app)
            .get()
            .is_anonymous_or_logged_out()
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        Container::new(
            Align::new(self.render_logout_button(appearance))
                .left()
                .finish(),
        )
        .with_margin_top(VERTICAL_MARGIN)
        .finish()
    }
}

impl SettingsPageMeta for MainSettingsPageView {
    fn section() -> SettingsSection {
        SettingsSection::Account
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        true
    }

    fn on_page_selected(&mut self, _: bool, ctx: &mut ViewContext<Self>) {
        // We want to immediately see if the user is part of a workspace rather than wait for the next poll.
        std::mem::drop(
            TeamUpdateManager::handle(ctx)
                .update(ctx, |manager, ctx| manager.refresh_workspace_metadata(ctx)),
        );
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<MainSettingsPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<MainSettingsPageView>) -> Self {
        SettingsPageViewHandle::Main(view_handle)
    }
}
