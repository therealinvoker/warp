use pathfinder_geometry::vector::vec2f;
use warp_completer::completer::CommandExitStatus;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Align, Border, ChildAnchor, Clipped, Container, CornerRadius, CrossAxisAlignment, Flex,
    MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius,
    Shrinkable, Stack, Text,
};
use warpui::presenter::ChildView;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use super::CloudObjectTypeAndId;
use crate::appearance::Appearance;
use crate::cloud_object::model::persistence::CloudModel;
use crate::editor::{EditorView, SingleLineEditorOptions};
use crate::marketplace_plugins::{CloudMarketplacePlugin, PluginSource};
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::SyncId;
use crate::terminal::model::session::LocalCommandExecutor;
use crate::terminal::shell::ShellType;

const MODAL_WIDTH: f32 = 500.;
const MODAL_CORNER_RADIUS: f32 = 8.;
const CLOSE_BUTTON_SIZE: f32 = 24.;
const HEADER_FONT_SIZE: f32 = 16.;
const LABEL_FONT_SIZE: f32 = 12.;
const INPUT_PADDING_HORIZONTAL: f32 = 12.;
const INPUT_PADDING_VERTICAL: f32 = 8.;
const BORDER_RADIUS_SMALL: f32 = 4.;
const BORDER_WIDTH: f32 = 1.;

#[derive(Debug)]
pub enum MarketplacePluginModalAction {
    Close,
    Save,
    Install,
}

pub enum MarketplacePluginModalEvent {
    Close,
}

/// Editor modal for a marketplace plugin drive object: edit the reference
/// (name, source, pinned version, description) and install the plugin locally
/// via the Cursor CLI. Opened by clicking a plugin item in the drive.
pub struct MarketplacePluginModal {
    /// The object being edited; `None` when the modal is closed.
    object_id: Option<SyncId>,
    name_editor: ViewHandle<EditorView>,
    source_editor: ViewHandle<EditorView>,
    version_editor: ViewHandle<EditorView>,
    description_editor: ViewHandle<EditorView>,
    close_button_mouse_state: MouseStateHandle,
    cancel_button_mouse_state: MouseStateHandle,
    save_button_mouse_state: MouseStateHandle,
    install_button_mouse_state: MouseStateHandle,
    /// Inline result line for the install action.
    install_status: Option<String>,
}

fn single_line_editor(
    placeholder: &str,
    ctx: &mut ViewContext<MarketplacePluginModal>,
) -> ViewHandle<EditorView> {
    let placeholder = placeholder.to_owned();
    ctx.add_typed_action_view(move |ctx| {
        let mut editor = EditorView::single_line(SingleLineEditorOptions::default(), ctx);
        editor.set_placeholder_text(&placeholder, ctx);
        editor
    })
}

impl MarketplacePluginModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        Self {
            object_id: None,
            name_editor: single_line_editor("Plugin name", ctx),
            source_editor: single_line_editor("publisher.extension or bundle URL", ctx),
            version_editor: single_line_editor("Pinned version (empty = latest)", ctx),
            description_editor: single_line_editor("Description", ctx),
            close_button_mouse_state: Default::default(),
            cancel_button_mouse_state: Default::default(),
            save_button_mouse_state: Default::default(),
            install_button_mouse_state: Default::default(),
            install_status: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.object_id.is_some()
    }

    /// Opens the modal for the given drive object. Returns false when the
    /// object can't be found (e.g. it was deleted by another client).
    pub fn open(&mut self, id: CloudObjectTypeAndId, ctx: &mut ViewContext<Self>) -> bool {
        let sync_id = id.sync_id();
        let Some(plugin) = Self::plugin_by_id(&sync_id, ctx) else {
            return false;
        };
        let model = plugin.model().string_model.clone();

        self.object_id = Some(sync_id);
        self.install_status = None;
        Self::set_editor_text(&self.name_editor, &model.name, ctx);
        Self::set_editor_text(&self.source_editor, model.source.display_identifier(), ctx);
        Self::set_editor_text(
            &self.version_editor,
            model.pinned_version.as_deref().unwrap_or(""),
            ctx,
        );
        Self::set_editor_text(
            &self.description_editor,
            model.description.as_deref().unwrap_or(""),
            ctx,
        );
        ctx.focus(&self.name_editor);
        ctx.notify();
        true
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        self.object_id = None;
        self.install_status = None;
        ctx.emit(MarketplacePluginModalEvent::Close);
        ctx.notify();
    }

    fn plugin_by_id<'a>(
        sync_id: &SyncId,
        app: &'a AppContext,
    ) -> Option<&'a CloudMarketplacePlugin> {
        let cloud_model = CloudModel::as_ref(app);
        let object = cloud_model.get_by_uid(&sync_id.uid())?;
        object.into()
    }

    fn set_editor_text(
        editor: &ViewHandle<EditorView>,
        text: &str,
        ctx: &mut ViewContext<MarketplacePluginModal>,
    ) {
        let text = text.to_owned();
        editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
            editor.set_buffer_text(text.as_str(), ctx);
        });
    }

    fn editor_text(editor: &ViewHandle<EditorView>, app: &AppContext) -> String {
        editor.as_ref(app).buffer_text(app).trim().to_owned()
    }

    /// Interprets the source field the same way creation does: URLs become
    /// bundle sources, anything else is a Cursor extension id.
    fn parse_source(text: &str) -> PluginSource {
        if text.starts_with("http://") || text.starts_with("https://") {
            PluginSource::Url {
                bundle_url: text.to_owned(),
            }
        } else {
            PluginSource::CursorExtension {
                extension_id: text.to_owned(),
            }
        }
    }

    fn save(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(sync_id) = self.object_id else {
            return;
        };
        let Some(plugin) = Self::plugin_by_id(&sync_id, ctx) else {
            self.install_status = Some("This plugin no longer exists.".to_owned());
            ctx.notify();
            return;
        };
        let mut model = plugin.model().string_model.clone();

        let name = Self::editor_text(&self.name_editor, ctx);
        let source = Self::editor_text(&self.source_editor, ctx);
        let version = Self::editor_text(&self.version_editor, ctx);
        let description = Self::editor_text(&self.description_editor, ctx);
        if name.is_empty() || source.is_empty() {
            self.install_status = Some("Name and source are required.".to_owned());
            ctx.notify();
            return;
        }

        model.name = name;
        model.source = Self::parse_source(&source);
        model.pinned_version = (!version.is_empty()).then_some(version);
        model.description = (!description.is_empty()).then_some(description);

        let revision = CloudModel::as_ref(ctx).current_revision(&sync_id).cloned();
        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            update_manager.update_marketplace_plugin(model, sync_id, revision, ctx);
        });
        self.close(ctx);
    }

    fn install(&mut self, ctx: &mut ViewContext<Self>) {
        let source = Self::editor_text(&self.source_editor, ctx);
        if source.is_empty() {
            self.install_status = Some("Enter a source to install.".to_owned());
            ctx.notify();
            return;
        }

        match Self::parse_source(&source) {
            PluginSource::Url { bundle_url } => {
                ctx.open_url(&bundle_url);
                self.install_status = Some("Opened the bundle URL in your browser.".to_owned());
                ctx.notify();
            }
            PluginSource::CursorExtension { extension_id } => {
                // Login shell so `cursor` resolves from the user's PATH even
                // when the app was launched from the dock.
                let escaped = format!("'{}'", extension_id.replace('\'', r"'\''"));
                let command = format!("cursor --install-extension {escaped}");
                self.install_status = Some(format!("Running: {command}"));
                ctx.notify();

                ctx.spawn(
                    async move {
                        let executor = LocalCommandExecutor::new(None, ShellType::Bash);
                        executor
                            .execute_local_command_in_login_shell(&command, None, None)
                            .await
                    },
                    |me, result, ctx| {
                        me.install_status = Some(match result {
                            Ok(output) if matches!(output.status, CommandExitStatus::Success) => {
                                "Installed via the Cursor CLI.".to_owned()
                            }
                            Ok(output) => {
                                let stderr = String::from_utf8_lossy(&output.stderr);
                                let detail = stderr.trim().lines().last().unwrap_or_default();
                                if detail.is_empty() {
                                    "The Cursor CLI reported a failure.".to_owned()
                                } else {
                                    format!("Cursor CLI failed: {detail}")
                                }
                            }
                            Err(err) => {
                                format!("Couldn't run the Cursor CLI (is Cursor installed?): {err}")
                            }
                        });
                        ctx.notify();
                    },
                );
            }
        }
    }

    fn render_close_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .close_button(CLOSE_BUTTON_SIZE, self.close_button_mouse_state.clone())
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(MarketplacePluginModalAction::Close))
            .finish()
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let top_row = Flex::row()
            .with_child(
                Shrinkable::new(
                    1.0,
                    Align::new(
                        Text::new_inline(
                            "Marketplace plugin",
                            appearance.ui_font_family(),
                            HEADER_FONT_SIZE,
                        )
                        .with_color(theme.active_ui_text_color().into())
                        .finish(),
                    )
                    .left()
                    .finish(),
                )
                .finish(),
            )
            .with_child(self.render_close_button(appearance))
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish();

        Container::new(top_row)
            .with_corner_radius(CornerRadius::with_top(Radius::Pixels(MODAL_CORNER_RADIUS)))
            .with_padding_left(24.)
            .with_padding_top(16.)
            .with_padding_right(16.)
            .with_padding_bottom(16.)
            .with_border(Border::bottom(1.).with_border_fill(theme.outline()))
            .finish()
    }

    fn render_labeled_input(
        &self,
        label: &str,
        editor: &ViewHandle<EditorView>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let label_text = appearance
            .ui_builder()
            .span(label.to_owned())
            .with_style(UiComponentStyles {
                font_size: Some(LABEL_FONT_SIZE),
                font_color: Some(theme.sub_text_color(theme.surface_2()).into()),
                ..Default::default()
            })
            .build()
            .finish();

        let input = Container::new(Clipped::new(ChildView::new(editor).finish()).finish())
            .with_margin_top(4.)
            .with_padding_top(INPUT_PADDING_VERTICAL)
            .with_padding_bottom(INPUT_PADDING_VERTICAL)
            .with_padding_left(INPUT_PADDING_HORIZONTAL)
            .with_padding_right(INPUT_PADDING_HORIZONTAL)
            .with_background(theme.background())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(BORDER_RADIUS_SMALL)))
            .with_border(Border::all(BORDER_WIDTH).with_border_fill(theme.outline()))
            .finish();

        Container::new(
            Flex::column()
                .with_child(label_text)
                .with_child(input)
                .finish(),
        )
        .with_margin_bottom(12.)
        .finish()
    }

    fn render_body(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut body = Flex::column()
            .with_child(self.render_labeled_input("Name", &self.name_editor, appearance))
            .with_child(self.render_labeled_input(
                "Source (Cursor extension id or bundle URL)",
                &self.source_editor,
                appearance,
            ))
            .with_child(self.render_labeled_input(
                "Pinned version",
                &self.version_editor,
                appearance,
            ))
            .with_child(self.render_labeled_input(
                "Description",
                &self.description_editor,
                appearance,
            ));

        if let Some(status) = &self.install_status {
            let theme = appearance.theme();
            body.add_child(
                appearance
                    .ui_builder()
                    .paragraph(status.clone())
                    .with_style(UiComponentStyles {
                        font_size: Some(LABEL_FONT_SIZE),
                        font_color: Some(theme.sub_text_color(theme.surface_2()).into()),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            );
        }

        Container::new(body.finish())
            .with_padding_left(24.)
            .with_padding_right(24.)
            .with_padding_top(16.)
            .finish()
    }

    fn render_footer(&self, appearance: &Appearance) -> Box<dyn Element> {
        let button_styles = UiComponentStyles {
            width: Some(110.),
            height: Some(36.),
            font_size: Some(14.),
            ..Default::default()
        };

        let cancel_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Outlined,
                self.cancel_button_mouse_state.clone(),
            )
            .with_centered_text_label("Cancel".to_owned())
            .with_style(button_styles)
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(MarketplacePluginModalAction::Close))
            .finish();

        let install_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Outlined,
                self.install_button_mouse_state.clone(),
            )
            .with_centered_text_label("Install".to_owned())
            .with_style(button_styles)
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(MarketplacePluginModalAction::Install))
            .finish();

        let save_button = appearance
            .ui_builder()
            .button(ButtonVariant::Accent, self.save_button_mouse_state.clone())
            .with_centered_text_label("Save".to_owned())
            .with_style(button_styles)
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(MarketplacePluginModalAction::Save))
            .finish();

        Container::new(
            Flex::row()
                .with_child(
                    Shrinkable::new(1.0, Align::new(cancel_button).left().finish()).finish(),
                )
                .with_child(
                    Container::new(install_button)
                        .with_margin_right(8.)
                        .finish(),
                )
                .with_child(save_button)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish(),
        )
        .with_uniform_padding(16.)
        .finish()
    }
}

impl Entity for MarketplacePluginModal {
    type Event = MarketplacePluginModalEvent;
}

impl View for MarketplacePluginModal {
    fn ui_name() -> &'static str {
        "MarketplacePluginModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let contents = Flex::column()
            .with_child(self.render_header(appearance))
            .with_child(self.render_body(appearance))
            .with_child(self.render_footer(appearance))
            .finish();

        let modal = warpui::elements::ConstrainedBox::new(
            Container::new(contents)
                .with_background(theme.surface_2())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(MODAL_CORNER_RADIUS)))
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .finish(),
        )
        .with_width(MODAL_WIDTH)
        .finish();

        let mut stack = Stack::new();
        stack.add_positioned_child(
            modal,
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::Center,
                ChildAnchor::Center,
            ),
        );

        Container::new(Align::new(stack.finish()).finish())
            .with_background_color(Fill::blur().into())
            .finish()
    }
}

impl TypedActionView for MarketplacePluginModal {
    type Action = MarketplacePluginModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            MarketplacePluginModalAction::Close => self.close(ctx),
            MarketplacePluginModalAction::Save => self.save(ctx),
            MarketplacePluginModalAction::Install => self.install(ctx),
        }
    }
}
