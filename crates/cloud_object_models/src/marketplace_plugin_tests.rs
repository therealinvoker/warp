use super::{MarketplacePlugin, PluginSource};

#[test]
fn test_from_user_input_extension_id() {
    let plugin = MarketplacePlugin::from_user_input("  dbaeumer.vscode-eslint ");
    assert_eq!(plugin.name, "dbaeumer.vscode-eslint");
    assert_eq!(
        plugin.source,
        PluginSource::CursorExtension {
            extension_id: "dbaeumer.vscode-eslint".to_string()
        }
    );
    assert_eq!(plugin.pinned_version, None);
}

#[test]
fn test_from_user_input_url() {
    let plugin =
        MarketplacePlugin::from_user_input("https://example.com/plugins/my-plugin.vsix");
    assert_eq!(plugin.name, "my-plugin.vsix");
    assert_eq!(
        plugin.source,
        PluginSource::Url {
            bundle_url: "https://example.com/plugins/my-plugin.vsix".to_string()
        }
    );
}

#[test]
fn test_serialization_round_trip() {
    let plugin = MarketplacePlugin::from_user_input("publisher.extension");
    let serialized = serde_json::to_string(&plugin).expect("serialize");
    let deserialized: MarketplacePlugin = serde_json::from_str(&serialized).expect("deserialize");
    assert_eq!(plugin, deserialized);
}

#[test]
fn test_deserialize_without_optional_fields_defaults() {
    // Objects written by future/other clients may omit optional fields;
    // ensure defaults apply.
    let json = r#"{"uuid":"00000000-0000-0000-0000-000000000000","name":"x","description":null}"#;
    let plugin: MarketplacePlugin = serde_json::from_str(json).expect("deserialize");
    assert_eq!(
        plugin.source,
        PluginSource::CursorExtension {
            extension_id: String::new()
        }
    );
    assert_eq!(plugin.pinned_version, None);
}
