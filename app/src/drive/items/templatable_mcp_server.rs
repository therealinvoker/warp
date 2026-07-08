use warpui::elements::{Container, Flex, MouseStateHandle, ParentElement};
use warpui::fonts::Weight;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element};

use super::{WarpDriveItem, WarpDriveItemId};
use crate::ai::mcp::templatable::CloudTemplatableMCPServer;
use crate::appearance::Appearance;
use crate::cloud_object::CloudObjectMetadata;
use crate::drive::index::DriveIndexAction;
use crate::drive::{CloudObjectTypeAndId, DriveObjectType};
use crate::themes::theme::Fill;

/// A templatable MCP server rendered as a regular drive item, so MCP servers
/// added to a personal or team drive show up alongside other objects.
#[derive(Clone)]
pub struct WarpDriveTemplatableMCPServer {
    id: CloudObjectTypeAndId,
    server: CloudTemplatableMCPServer,
}

impl WarpDriveTemplatableMCPServer {
    pub fn new(id: CloudObjectTypeAndId, server: CloudTemplatableMCPServer) -> Self {
        Self { id, server }
    }
}

impl WarpDriveItem for WarpDriveTemplatableMCPServer {
    fn display_name(&self) -> Option<String> {
        Some(self.server.model().string_model.name.clone())
    }

    fn metadata(&self) -> Option<&CloudObjectMetadata> {
        Some(&self.server.metadata)
    }

    fn object_type(&self) -> Option<DriveObjectType> {
        Some(DriveObjectType::MCPServer)
    }

    fn secondary_icon(&self, _color: Option<Fill>) -> Option<Box<dyn Element>> {
        None
    }

    fn click_action(&self) -> Option<DriveIndexAction> {
        // Management (install, edit, start/stop) lives in the MCP servers
        // pane; route clicks there.
        Some(DriveIndexAction::OpenMCPServerCollection)
    }

    fn preview(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let model = &self.server.model().string_model;
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

        if let Some(description) = model.description.clone() {
            let description_text =
                appearance
                    .ui_builder()
                    .paragraph(description)
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
                Container::new(description_text.build().finish())
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
        self.server.metadata.pending_changes_statuses.render_icon(
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
