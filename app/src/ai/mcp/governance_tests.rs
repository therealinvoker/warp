use uuid::Uuid;
use warpui::App;

use super::{EffectiveMcpMode, EffectiveMcpPolicy, McpGovernance};
use crate::ai::mcp::ServerOrigin;
use crate::workspaces::workspace::{
    MarketplacePolicy, McpAllowlistEntry, McpAllowlistEntryKind, McpGovernanceMode,
    McpGovernanceSettings, Workspace,
};

/// Builds a workspace with the given governance entitlement + settings.
fn workspace(
    name: &str,
    governance_controls_enabled: Option<bool>,
    settings: Option<McpGovernanceSettings>,
) -> Workspace {
    let mut workspace =
        Workspace::from_local_cache(format!("uid-{name}").into(), name.to_string(), None);
    workspace.billing_metadata.tier.marketplace_policy =
        governance_controls_enabled.map(|enabled| MarketplacePolicy {
            enabled: true,
            governance_controls_enabled: enabled,
            org_sources_enabled: false,
            max_org_sources: 0,
        });
    workspace.settings.mcp_governance_settings = settings;
    workspace
}

fn governance_settings(mode: McpGovernanceMode) -> McpGovernanceSettings {
    McpGovernanceSettings {
        mode,
        allowlist: vec![],
        allow_file_based_servers: true,
        allow_plugin_import: true,
    }
}

fn allowlist_entry(value: &str) -> McpAllowlistEntry {
    McpAllowlistEntry {
        id: format!("id-{value}"),
        kind: McpAllowlistEntryKind::RegistryName,
        value: value.to_string(),
        pinned_version: None,
        display_name: None,
    }
}

#[test]
fn solo_user_with_no_workspaces_resolves_to_enable_all() {
    let policy = EffectiveMcpPolicy::resolve(&[]);
    assert_eq!(policy, EffectiveMcpPolicy::self_managed());
    assert_eq!(policy.mode, EffectiveMcpMode::EnableAll);
    assert!(policy.allow_file_based_servers);
}

#[test]
fn workspace_with_null_settings_is_self_managed() {
    let workspaces = vec![workspace("a", Some(true), None)];
    let policy = EffectiveMcpPolicy::resolve(&workspaces);
    assert_eq!(policy, EffectiveMcpPolicy::self_managed());
}

#[test]
fn single_team_enable_all_resolves_to_enable_all() {
    let workspaces = vec![workspace(
        "a",
        Some(true),
        Some(governance_settings(McpGovernanceMode::EnableAll)),
    )];
    let policy = EffectiveMcpPolicy::resolve(&workspaces);
    assert_eq!(policy.mode, EffectiveMcpMode::EnableAll);
    assert!(policy.allow_file_based_servers);
}

#[test]
fn single_team_disable_resolves_to_disable() {
    let workspaces = vec![workspace(
        "a",
        Some(true),
        Some(governance_settings(McpGovernanceMode::Disable)),
    )];
    let policy = EffectiveMcpPolicy::resolve(&workspaces);
    assert_eq!(policy.mode, EffectiveMcpMode::Disable);
}

#[test]
fn multi_team_most_restrictive_wins_disable_anywhere() {
    let workspaces = vec![
        workspace(
            "permissive",
            Some(true),
            Some(governance_settings(McpGovernanceMode::EnableAll)),
        ),
        workspace(
            "restrictive",
            Some(true),
            Some(governance_settings(McpGovernanceMode::Disable)),
        ),
    ];
    let policy = EffectiveMcpPolicy::resolve(&workspaces);
    assert_eq!(policy.mode, EffectiveMcpMode::Disable);
}

#[test]
fn tier_without_governance_entitlement_is_ignored() {
    // A DISABLE setting on a workspace whose tier lacks
    // governance_controls_enabled must be ignored entirely.
    let workspaces = vec![
        workspace(
            "not-entitled",
            Some(false),
            Some(governance_settings(McpGovernanceMode::Disable)),
        ),
        workspace(
            "no-marketplace-policy",
            None,
            Some(governance_settings(McpGovernanceMode::Disable)),
        ),
    ];
    let policy = EffectiveMcpPolicy::resolve(&workspaces);
    assert_eq!(policy, EffectiveMcpPolicy::self_managed());
}

#[test]
fn allowlist_workspaces_combine_by_intersection() {
    let mut settings_a = governance_settings(McpGovernanceMode::Allowlist);
    settings_a.allowlist = vec![allowlist_entry("a")];
    let mut settings_b = governance_settings(McpGovernanceMode::Allowlist);
    settings_b.allowlist = vec![allowlist_entry("b")];

    let workspaces = vec![
        workspace("a", Some(true), Some(settings_a)),
        workspace("b", Some(true), Some(settings_b)),
        // EnableAll doesn't relax the allowlist mode.
        workspace(
            "c",
            Some(true),
            Some(governance_settings(McpGovernanceMode::EnableAll)),
        ),
    ];
    let policy = EffectiveMcpPolicy::resolve(&workspaces);
    match &policy.mode {
        EffectiveMcpMode::Allowlist { allowlists } => {
            assert_eq!(allowlists.len(), 2);
            assert_eq!(allowlists[0][0].value, "a");
            assert_eq!(allowlists[1][0].value, "b");
        }
        other => panic!("expected Allowlist mode, got {other:?}"),
    }
}

#[test]
fn allow_file_based_servers_is_anded_across_workspaces() {
    let mut restrictive = governance_settings(McpGovernanceMode::EnableAll);
    restrictive.allow_file_based_servers = false;

    let workspaces = vec![
        workspace(
            "permissive",
            Some(true),
            Some(governance_settings(McpGovernanceMode::EnableAll)),
        ),
        workspace("restrictive", Some(true), Some(restrictive)),
    ];
    let policy = EffectiveMcpPolicy::resolve(&workspaces);
    assert_eq!(policy.mode, EffectiveMcpMode::EnableAll);
    assert!(!policy.allow_file_based_servers);
}

#[test]
fn disable_blocks_spawn_for_every_origin() {
    let policy = EffectiveMcpPolicy {
        mode: EffectiveMcpMode::Disable,
        allow_file_based_servers: true,
    };
    for origin in [
        ServerOrigin::Manual,
        ServerOrigin::Gallery,
        ServerOrigin::CursorImport,
        ServerOrigin::Registry,
        ServerOrigin::OrgMarketplace,
        ServerOrigin::FileBased,
    ] {
        assert!(!policy.allows_spawn(origin), "{origin:?} should be blocked");
    }
    assert!(!policy.allows_new_installs());
}

#[test]
fn enable_all_with_file_based_disallowed_blocks_only_file_based() {
    let policy = EffectiveMcpPolicy {
        mode: EffectiveMcpMode::EnableAll,
        allow_file_based_servers: false,
    };
    assert!(policy.allows_spawn(ServerOrigin::Manual));
    assert!(policy.allows_spawn(ServerOrigin::Gallery));
    assert!(policy.allows_spawn(ServerOrigin::CursorImport));
    assert!(!policy.allows_spawn(ServerOrigin::FileBased));
    assert!(policy.allows_new_installs());
}

#[test]
fn allowlist_mode_is_stubbed_to_allow_all_in_m1() {
    // TODO(M2): tighten once candidate matching lands.
    let policy = EffectiveMcpPolicy {
        mode: EffectiveMcpMode::Allowlist {
            allowlists: vec![vec![allowlist_entry("only-this")]],
        },
        allow_file_based_servers: true,
    };
    assert!(policy.allows_spawn(ServerOrigin::Manual));
    assert!(policy.allows_spawn(ServerOrigin::FileBased));
    assert!(policy.allows_new_installs());
}

#[test]
fn compute_shutdown_set_selects_only_disallowed_servers() {
    let manual_uuid = Uuid::new_v4();
    let file_based_uuid = Uuid::new_v4();
    let running = [
        (manual_uuid, ServerOrigin::Manual),
        (file_based_uuid, ServerOrigin::FileBased),
    ];

    // File-based disallowed: only the file-based server is shut down.
    let policy = EffectiveMcpPolicy {
        mode: EffectiveMcpMode::EnableAll,
        allow_file_based_servers: false,
    };
    assert_eq!(policy.compute_shutdown_set(running), vec![file_based_uuid]);

    // Kill switch: everything is shut down.
    let policy = EffectiveMcpPolicy {
        mode: EffectiveMcpMode::Disable,
        allow_file_based_servers: true,
    };
    let mut shutdown = policy.compute_shutdown_set(running);
    shutdown.sort();
    let mut expected = vec![manual_uuid, file_based_uuid];
    expected.sort();
    assert_eq!(shutdown, expected);

    // Fully permissive: nothing is shut down (and loosening never restarts,
    // since this is the only policy-change action).
    let policy = EffectiveMcpPolicy::self_managed();
    assert!(policy.compute_shutdown_set(running).is_empty());
}

#[test]
fn policy_snapshot_round_trips_through_json() {
    let policy = EffectiveMcpPolicy {
        mode: EffectiveMcpMode::Allowlist {
            allowlists: vec![vec![allowlist_entry("registry-name")]],
        },
        allow_file_based_servers: false,
    };
    let json = serde_json::to_string(&policy).expect("serialize");
    let round_tripped: EffectiveMcpPolicy = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(round_tripped, policy);
}

#[test]
fn model_loads_cached_snapshot_at_startup() {
    let cached = EffectiveMcpPolicy {
        mode: EffectiveMcpMode::Disable,
        allow_file_based_servers: false,
    };
    let cached_json = serde_json::to_string(&cached).expect("serialize");

    App::test((), |mut app| async move {
        let handle =
            app.add_singleton_model(move |ctx| McpGovernance::new(Some(cached_json), ctx));
        handle.read(&app, |governance, _| {
            assert_eq!(governance.effective_policy(), &cached);
        });
    });
}

#[test]
fn model_falls_back_to_self_managed_without_or_with_corrupt_cache() {
    App::test((), |mut app| async move {
        let no_cache = app.add_singleton_model(|ctx| McpGovernance::new(None, ctx));
        no_cache.read(&app, |governance, _| {
            assert_eq!(
                governance.effective_policy(),
                &EffectiveMcpPolicy::self_managed()
            );
        });
    });

    App::test((), |mut app| async move {
        let corrupt_cache = app
            .add_singleton_model(|ctx| McpGovernance::new(Some("not json".to_string()), ctx));
        corrupt_cache.read(&app, |governance, _| {
            assert_eq!(
                governance.effective_policy(),
                &EffectiveMcpPolicy::self_managed()
            );
        });
    });
}
