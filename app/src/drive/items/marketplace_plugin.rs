use warpui::elements::{Container, Flex, MouseStateHandle, ParentElement};
use warpui::fonts::Weight;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element};

use super::{WarpDriveItem, WarpDriveItemId};
use crate::appearance::Appearance;
use crate::cloud_object::CloudObjectMetadata;
use crate::drive::index::DriveIndexAction;
use crate::drive::{CloudObjectTypeAndId, DriveObjectType};
use crate::marketplace_plugins::{CloudMarketplacePlugin, PluginSource};
use crate::themes::theme::Fill;

#[derive(Clone)]
pub struct WarpDriveMarketplacePlugin {
    id: CloudObjectTypeAndId,
    plugin: CloudMarketplacePlugin,
}

impl WarpDriveMarketplacePlugin {
    pub fn new(id: CloudObjectTypeAndId, plugin: CloudMarketplacePlugin) -> Self {
        Self { id, plugin }
    }
}

impl WarpDriveItem for WarpDriveMarketplacePlugin {
    fn display_name(&self) -> Option<String> {
        Some(self.plugin.model().string_model.name.clone())
    }

    fn metadata(&self) -> Option<&CloudObjectMetadata> {
        Some(&self.plugin.metadata)
    }

    fn object_type(&self) -> Option<DriveObjectType> {
        Some(DriveObjectType::MarketplacePlugin)
    }

    fn secondary_icon(&self, _color: Option<Fill>) -> Option<Box<dyn Element>> {
        None
    }

    fn click_action(&self) -> Option<DriveIndexAction> {
        Some(DriveIndexAction::OpenObject(self.id))
    }

    fn preview(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let model = &self.plugin.model().string_model;
        let title = appearance
            .ui_builder()
            .wrappable_text(model.name.clone(), true)
            .with_style(UiComponentStyles {
                font_color: Some(
                    appearance
                        .theme()
                        .main_text_color(appearance.theme().background())
                        .into(),
                ),
                font_size: Some(14.),
                font_weight: Some(Weight::Bold),
                ..Default::default()
            })
            .build()
            .finish();

        let mut text = Flex::column().with_child(Container::new(title).finish());

        let source_label = match &model.source {
            PluginSource::CursorExtension { extension_id } => {
                format!("Cursor extension: {extension_id}")
            }
            PluginSource::Url { bundle_url } => format!("Bundle: {bundle_url}"),
        };
        let mut lines = vec![source_label];
        if let Some(description) = model.description.clone() {
            lines.insert(0, description);
        }
        if let Some(version) = model.pinned_version.clone() {
            lines.push(format!("Pinned version: {version}"));
        }

        for line in lines {
            let line_text = appearance
                .ui_builder()
                .paragraph(line)
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_color: Some(
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().surface_2())
                            .into(),
                    ),
                    font_size: Some(12.),
                    ..Default::default()
                });
            text.add_child(
                Container::new(line_text.build().finish())
                    .with_margin_top(4.)
                    .finish(),
            );
        }

        Some(text.finish())
    }

    fn warp_drive_id(&self) -> WarpDriveItemId {
        WarpDriveItemId::Object(self.id)
    }

    fn sync_status_icon(
        &self,
        sync_queue_is_dequeueing: bool,
        hover_state: MouseStateHandle,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        self.plugin.metadata.pending_changes_statuses.render_icon(
            sync_queue_is_dequeueing,
            hover_state,
            appearance,
        )
    }

    fn action_summary(&self, _app: &AppContext) -> Option<String> {
        None
    }

    fn clone_box(&self) -> Box<dyn WarpDriveItem> {
        Box::new(self.clone())
    }
}
