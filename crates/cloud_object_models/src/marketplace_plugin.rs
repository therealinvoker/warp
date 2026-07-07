use cloud_objects::cloud_object::{
    GenericCloudObject, GenericServerObject, GenericStringModel, JsonObjectType,
};
use cloud_objects::ids::GenericStringObjectId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::mcp::ServerOrigin;
use crate::{JsonModel, JsonSerializer};

/// Where a marketplace plugin's code comes from.
///
/// Serialized as part of the [`MarketplacePlugin`] JSON blob, so new variants
/// can be added without a storage schema change. Old clients that don't know a
/// variant fail deserialization of that one object and skip it, matching the
/// behavior of other generic string objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginSource {
    /// A Cursor-compatible extension identified by its marketplace id, e.g.
    /// `publisher.extension-name`.
    CursorExtension { extension_id: String },
    /// A direct URL to a plugin bundle (e.g. a `.vsix` or plugin archive).
    Url { bundle_url: String },
}

impl Default for PluginSource {
    fn default() -> Self {
        PluginSource::CursorExtension {
            extension_id: String::new(),
        }
    }
}

impl PluginSource {
    /// A short human-readable identifier for the source, used in list UIs.
    pub fn display_identifier(&self) -> &str {
        match self {
            PluginSource::CursorExtension { extension_id } => extension_id,
            PluginSource::Url { bundle_url } => bundle_url,
        }
    }
}

/// A marketplace plugin (e.g. a Cursor extension) stored as a Warp Drive
/// object so it can live in the personal drive or a team drive and sync
/// through the regular cloud object pipeline.
///
/// The drive object is the *reference* to the plugin (identity, source,
/// pinned version); installation/activation stays a per-user, per-device
/// concern, mirroring how [`crate::mcp::TemplatableMCPServer`] templates are
/// installed locally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MarketplacePlugin {
    pub uuid: Uuid,
    /// Display name shown in the drive.
    pub name: String,
    pub description: Option<String>,
    /// Where the plugin comes from.
    #[serde(default)]
    pub source: PluginSource,
    /// Optional pinned version. `None` means "latest".
    #[serde(default)]
    pub pinned_version: Option<String>,
    /// How this plugin entered Warp (manual add, gallery, org marketplace, ...).
    /// Reuses the MCP provenance enum so governance treats both uniformly.
    #[serde(default)]
    pub origin: ServerOrigin,
}

impl MarketplacePlugin {
    /// Creates a plugin reference from user free-text input.
    ///
    /// Input that looks like a URL becomes a [`PluginSource::Url`]; anything
    /// else is treated as a Cursor marketplace extension id. The display name
    /// defaults to the last meaningful path/id segment.
    pub fn from_user_input(input: &str) -> Self {
        let input = input.trim();
        let (name, source) = if input.starts_with("http://") || input.starts_with("https://") {
            let name = input
                .rsplit('/')
                .find(|segment| !segment.is_empty())
                .unwrap_or(input)
                .to_owned();
            (
                name,
                PluginSource::Url {
                    bundle_url: input.to_owned(),
                },
            )
        } else {
            (
                input.to_owned(),
                PluginSource::CursorExtension {
                    extension_id: input.to_owned(),
                },
            )
        };
        MarketplacePlugin {
            uuid: Uuid::new_v4(),
            name,
            description: None,
            source,
            pinned_version: None,
            origin: ServerOrigin::Manual,
        }
    }
}

impl JsonModel for MarketplacePlugin {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::MarketplacePlugin
    }
}

pub type CloudMarketplacePlugin =
    GenericCloudObject<GenericStringObjectId, CloudMarketplacePluginModel>;
pub type CloudMarketplacePluginModel = GenericStringModel<MarketplacePlugin, JsonSerializer>;
pub type ServerMarketplacePlugin =
    GenericServerObject<GenericStringObjectId, CloudMarketplacePluginModel>;

#[cfg(test)]
#[path = "marketplace_plugin_tests.rs"]
mod tests;
