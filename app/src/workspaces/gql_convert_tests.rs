use warp_graphql::billing::MarketplacePolicy as GqlMarketplacePolicy;
use warp_graphql::workspace::{
    McpAllowlistEntry as GqlMcpAllowlistEntry, McpAllowlistEntryKind as GqlMcpAllowlistEntryKind,
    McpGovernanceMode as GqlMcpGovernanceMode, McpGovernanceSettings as GqlMcpGovernanceSettings,
};

use super::workspace::{MarketplacePolicy, McpAllowlistEntryKind, McpGovernanceMode};

fn gql_entry(kind: GqlMcpAllowlistEntryKind) -> GqlMcpAllowlistEntry {
    GqlMcpAllowlistEntry {
        id: cynic::Id::new("entry-1"),
        kind,
        value: "value-1".to_string(),
        pinned_version: Some("1.2.3".to_string()),
        display_name: Some("Display".to_string()),
    }
}

#[test]
fn converts_marketplace_policy() {
    let policy: MarketplacePolicy = GqlMarketplacePolicy {
        enabled: true,
        governance_controls_enabled: true,
        org_sources_enabled: false,
        max_org_sources: 5,
    }
    .into();

    assert!(policy.enabled);
    assert!(policy.governance_controls_enabled);
    assert!(!policy.org_sources_enabled);
    assert_eq!(policy.max_org_sources, 5);
}

#[test]
fn converts_mcp_governance_settings() {
    let settings: super::workspace::McpGovernanceSettings = GqlMcpGovernanceSettings {
        mode: GqlMcpGovernanceMode::Allowlist,
        allowlist: vec![gql_entry(GqlMcpAllowlistEntryKind::RegistryName)],
        allow_file_based_servers: false,
        allow_plugin_import: true,
    }
    .into();

    assert_eq!(settings.mode, McpGovernanceMode::Allowlist);
    assert!(!settings.allow_file_based_servers);
    assert!(settings.allow_plugin_import);
    assert_eq!(settings.allowlist.len(), 1);
    let entry = &settings.allowlist[0];
    assert_eq!(entry.id, "entry-1");
    assert_eq!(entry.kind, McpAllowlistEntryKind::RegistryName);
    assert_eq!(entry.value, "value-1");
    assert_eq!(entry.pinned_version.as_deref(), Some("1.2.3"));
    assert_eq!(entry.display_name.as_deref(), Some("Display"));
}

#[test]
fn converts_all_known_governance_modes() {
    assert_eq!(
        McpGovernanceMode::from(GqlMcpGovernanceMode::Disable),
        McpGovernanceMode::Disable
    );
    assert_eq!(
        McpGovernanceMode::from(GqlMcpGovernanceMode::EnableAll),
        McpGovernanceMode::EnableAll
    );
    assert_eq!(
        McpGovernanceMode::from(GqlMcpGovernanceMode::Allowlist),
        McpGovernanceMode::Allowlist
    );
}

#[test]
fn unknown_governance_mode_fails_closed_to_disable() {
    let mode = McpGovernanceMode::from(GqlMcpGovernanceMode::Other("SOMETHING_NEW".to_string()));
    assert_eq!(mode, McpGovernanceMode::Disable);
}

#[test]
fn unknown_allowlist_entry_kind_drops_the_entry() {
    let settings: super::workspace::McpGovernanceSettings = GqlMcpGovernanceSettings {
        mode: GqlMcpGovernanceMode::Allowlist,
        allowlist: vec![
            gql_entry(GqlMcpAllowlistEntryKind::CanonicalHash),
            gql_entry(GqlMcpAllowlistEntryKind::Other("NEW_KIND".to_string())),
        ],
        allow_file_based_servers: true,
        allow_plugin_import: false,
    }
    .into();

    assert_eq!(settings.allowlist.len(), 1);
    assert_eq!(
        settings.allowlist[0].kind,
        McpAllowlistEntryKind::CanonicalHash
    );
}

#[test]
fn converts_all_known_allowlist_entry_kinds() {
    let cases = [
        (
            GqlMcpAllowlistEntryKind::RegistryName,
            McpAllowlistEntryKind::RegistryName,
        ),
        (
            GqlMcpAllowlistEntryKind::GalleryTemplate,
            McpAllowlistEntryKind::GalleryTemplate,
        ),
        (
            GqlMcpAllowlistEntryKind::OrgMarketplaceEntry,
            McpAllowlistEntryKind::OrgMarketplaceEntry,
        ),
        (
            GqlMcpAllowlistEntryKind::UrlPattern,
            McpAllowlistEntryKind::UrlPattern,
        ),
        (
            GqlMcpAllowlistEntryKind::CommandPattern,
            McpAllowlistEntryKind::CommandPattern,
        ),
        (
            GqlMcpAllowlistEntryKind::CanonicalHash,
            McpAllowlistEntryKind::CanonicalHash,
        ),
    ];
    for (gql_kind, expected) in cases {
        let settings: super::workspace::McpGovernanceSettings = GqlMcpGovernanceSettings {
            mode: GqlMcpGovernanceMode::EnableAll,
            allowlist: vec![gql_entry(gql_kind)],
            allow_file_based_servers: true,
            allow_plugin_import: true,
        }
        .into();
        assert_eq!(settings.allowlist[0].kind, expected);
    }
}
