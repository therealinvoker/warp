//! App-side integration for marketplace plugin drive objects (e.g. Cursor
//! extensions). The payload model lives in `cloud_object_models`; this module
//! wires it into the app's cloud object machinery (sync queue, drive
//! rendering).

pub use cloud_object_models::{
    CloudMarketplacePlugin, CloudMarketplacePluginModel, MarketplacePlugin, PluginSource,
};
use warp_core::ui::appearance::Appearance;

use crate::cloud_object::model::generic_string_model::StringModel;
use crate::cloud_object::model::json_model::JsonModel;
use crate::cloud_object::{
    CloudObjectUuid, GenericStringObjectFormat, GenericStringObjectUniqueKey, JsonObjectType,
    Revision, UniquePer,
};
use crate::drive::items::marketplace_plugin::WarpDriveMarketplacePlugin;
use crate::drive::items::WarpDriveItem;
use crate::drive::CloudObjectTypeAndId;
use crate::server::ids::SyncId;
use crate::server::sync_queue::QueueItem;

const UNIQUENESS_KEY_PREFIX: &str = "marketplace_plugin";

impl CloudObjectUuid for MarketplacePlugin {
    fn uuid(&self) -> uuid::Uuid {
        self.uuid
    }
}

impl StringModel for MarketplacePlugin {
    type CloudObjectType = CloudMarketplacePlugin;

    fn model_type_name(&self) -> &'static str {
        "Marketplace plugin"
    }

    fn should_enforce_revisions() -> bool {
        true
    }

    fn model_format() -> GenericStringObjectFormat {
        GenericStringObjectFormat::Json(JsonObjectType::MarketplacePlugin)
    }

    fn should_show_activity_toasts() -> bool {
        true
    }

    fn warn_if_unsaved_at_quit() -> bool {
        true
    }

    fn display_name(&self) -> String {
        self.name.clone()
    }

    fn set_display_name(&mut self, name: &str) {
        self.name = name.to_owned();
    }

    fn update_object_queue_item(
        &self,
        revision_ts: Option<Revision>,
        object: &Self::CloudObjectType,
    ) -> QueueItem {
        QueueItem::UpdateMarketplacePlugin {
            model: object.model().clone().into(),
            id: object.id,
            revision: revision_ts.or_else(|| object.metadata.revision.clone()),
        }
    }

    fn uniqueness_key(&self) -> Option<GenericStringObjectUniqueKey> {
        Some(GenericStringObjectUniqueKey {
            key: format!("{UNIQUENESS_KEY_PREFIX}_{}", self.uuid),
            unique_per: UniquePer::User,
        })
    }

    fn renders_in_warp_drive(&self) -> bool {
        true
    }

    fn to_warp_drive_item(
        &self,
        id: SyncId,
        _appearance: &Appearance,
        plugin: &CloudMarketplacePlugin,
    ) -> Option<Box<dyn WarpDriveItem>> {
        Some(Box::new(WarpDriveMarketplacePlugin::new(
            CloudObjectTypeAndId::GenericStringObject {
                object_type: GenericStringObjectFormat::Json(JsonObjectType::MarketplacePlugin),
                id,
            },
            plugin.clone(),
        )))
    }
}

impl JsonModel for MarketplacePlugin {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::MarketplacePlugin
    }
}
