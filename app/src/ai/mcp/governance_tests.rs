use uuid::Uuid;
use warpui::App;

use super::{
    normalize_url, CandidateSpec, EffectiveMcpMode, EffectiveMcpPolicy, McpGovernance,
    ServerCandidate,
};
use crate::ai::mcp::{ServerOrigin, TemplatableMCPServer};
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
    // ServerIds must be exactly 22 characters; pad/truncate the name to fit.
    let padded = format!("{name:0>22}");
    let uid = padded[padded.len() - 22..].to_string();
    let mut workspace = Workspace::from_local_cache(uid.into(), name.to_string(), None);
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
    entry(McpAllowlistEntryKind::RegistryName, value)
}

fn entry(kind: McpAllowlistEntryKind, value: &str) -> McpAllowlistEntry {
    McpAllowlistEntry {
        id: format!("id-{value}"),
        kind,
        value: value.to_string(),
        pinned_version: None,
        display_name: None,
    }
}

fn stdio_candidate(name: &str, command: &str) -> ServerCandidate {
    ServerCandidate {
        name: Some(name.to_string()),
        spec: Some(CandidateSpec::Stdio {
            command: command.to_string(),
            args: vec![],
            env_keys: vec![],
        }),
        ..Default::default()
    }
}

fn remote_candidate(name: &str, url: &str) -> ServerCandidate {
    ServerCandidate {
        name: Some(name.to_string()),
        spec: Some(CandidateSpec::Remote {
            url: url.to_string(),
        }),
        ..Default::default()
    }
}

/// Builds an allowlist-mode policy from one entry list per governed
/// workspace.
fn allowlist_policy(allowlists: Vec<Vec<McpAllowlistEntry>>) -> EffectiveMcpPolicy {
    EffectiveMcpPolicy {
        mode: EffectiveMcpMode::Allowlist { allowlists },
        allow_file_based_servers: true,
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
        assert!(
            !policy.allows_spawn(origin, None),
            "{origin:?} should be blocked"
        );
    }
    assert!(!policy.allows_new_installs());
    assert!(!policy.allows_install(&stdio_candidate("anything", "npx")));
}

#[test]
fn enable_all_with_file_based_disallowed_blocks_only_file_based() {
    let policy = EffectiveMcpPolicy {
        mode: EffectiveMcpMode::EnableAll,
        allow_file_based_servers: false,
    };
    assert!(policy.allows_spawn(ServerOrigin::Manual, None));
    assert!(policy.allows_spawn(ServerOrigin::Gallery, None));
    assert!(policy.allows_spawn(ServerOrigin::CursorImport, None));
    assert!(!policy.allows_spawn(ServerOrigin::FileBased, None));
    assert!(policy.allows_new_installs());
}

#[test]
fn compute_shutdown_set_selects_only_disallowed_servers() {
    let manual_uuid = Uuid::new_v4();
    let file_based_uuid = Uuid::new_v4();
    let running = || {
        [
            (manual_uuid, ServerOrigin::Manual, None),
            (file_based_uuid, ServerOrigin::FileBased, None),
        ]
    };

    // File-based disallowed: only the file-based server is shut down.
    let policy = EffectiveMcpPolicy {
        mode: EffectiveMcpMode::EnableAll,
        allow_file_based_servers: false,
    };
    assert_eq!(
        policy.compute_shutdown_set(running()),
        vec![file_based_uuid]
    );

    // Kill switch: everything is shut down.
    let policy = EffectiveMcpPolicy {
        mode: EffectiveMcpMode::Disable,
        allow_file_based_servers: true,
    };
    let mut shutdown = policy.compute_shutdown_set(running());
    shutdown.sort();
    let mut expected = vec![manual_uuid, file_based_uuid];
    expected.sort();
    assert_eq!(shutdown, expected);

    // Fully permissive: nothing is shut down (and loosening never restarts,
    // since this is the only policy-change action).
    let policy = EffectiveMcpPolicy::self_managed();
    assert!(policy.compute_shutdown_set(running()).is_empty());
}

#[test]
fn compute_shutdown_set_evaluates_candidates_under_allowlist() {
    let allowed_uuid = Uuid::new_v4();
    let blocked_uuid = Uuid::new_v4();
    let policy = allowlist_policy(vec![vec![allowlist_entry("allowed-server")]]);

    let running = [
        (
            allowed_uuid,
            ServerOrigin::Manual,
            Some(stdio_candidate("allowed-server", "npx")),
        ),
        (
            blocked_uuid,
            ServerOrigin::Manual,
            Some(stdio_candidate("blocked-server", "npx")),
        ),
    ];
    assert_eq!(policy.compute_shutdown_set(running), vec![blocked_uuid]);
}

// ---------------------------------------------------------------------------
// Canonical hash contract vectors — generated from harness-backend
// `src/mcp/identity.js` (`canonicalString` / `canonicalHash`). Do NOT edit the
// expected values by hand; regenerate them from identity.js if the contract
// ever changes (it must change on both sides simultaneously).
// ---------------------------------------------------------------------------

#[test]
fn canonical_hash_matches_identity_js_stdio_vectors() {
    let spec = CandidateSpec::Stdio {
        command: "npx".to_string(),
        args: vec![
            "-y".to_string(),
            "@modelcontextprotocol/server-filesystem".to_string(),
            "/tmp".to_string(),
        ],
        // Deliberately unsorted: keys are sorted lexicographically as part of
        // canonicalization.
        env_keys: vec!["HOME".to_string(), "API_KEY".to_string()],
    };
    assert_eq!(
        spec.canonical_string(),
        r#"["stdio","npx",["-y","@modelcontextprotocol/server-filesystem","/tmp"],["API_KEY","HOME"]]"#
    );
    assert_eq!(
        spec.canonical_hash(),
        "9ad2865b1ece870ec1dc40a327feb61be918fc9fc63430099dab54966f8ed549"
    );

    let empty = CandidateSpec::Stdio {
        command: "uvx".to_string(),
        args: vec![],
        env_keys: vec![],
    };
    assert_eq!(empty.canonical_string(), r#"["stdio","uvx",[],[]]"#);
    assert_eq!(
        empty.canonical_hash(),
        "151998f462d22af46d03c1d7c3e9275b702dce0d79a1cb914477599512d53d6a"
    );

    let unsorted_env = CandidateSpec::Stdio {
        command: "docker".to_string(),
        args: vec!["run".to_string(), "--rm".to_string(), "img".to_string()],
        env_keys: vec!["ZED".to_string(), "ALPHA".to_string(), "MID".to_string()],
    };
    assert_eq!(
        unsorted_env.canonical_string(),
        r#"["stdio","docker",["run","--rm","img"],["ALPHA","MID","ZED"]]"#
    );
    assert_eq!(
        unsorted_env.canonical_hash(),
        "90c12ecadba13eaa4e58b3723fab3dd7ebc97734d3d161166c8f8684e32504f6"
    );
}

#[test]
fn canonical_hash_matches_identity_js_remote_and_plugin_vectors() {
    let cases: Vec<(CandidateSpec, &str, &str)> = vec![
        (
            CandidateSpec::Remote {
                url: "HTTPS://Example.COM:443/mcp/".to_string(),
            },
            r#"["remote","https://example.com/mcp"]"#,
            "a22ee4e3a1dcb953f93cd15ffcb3b9610e01c54e8d797e2d56756dd636f3bc50",
        ),
        (
            CandidateSpec::Remote {
                url: "https://example.com/mcp?b=2&a=1#frag".to_string(),
            },
            r#"["remote","https://example.com/mcp?b=2&a=1"]"#,
            "e6cac95c2bd3838305e8f92d9ed528d16dcbcab34517f3b60e5a53958898e89f",
        ),
        (
            CandidateSpec::Remote {
                url: "http://EXAMPLE.com:80/".to_string(),
            },
            r#"["remote","http://example.com"]"#,
            "b67472c63f1f60224cc305f4936fa5e7bd7e82dabe27b8aaa7b494c248b9a60c",
        ),
        (
            CandidateSpec::Remote {
                url: "not a url at all  ".to_string(),
            },
            r#"["remote","not a url at all"]"#,
            "b43ca8c0900ba402ea79de9a15824c5395e057bd89f17042ffc9caa8b0a30623",
        ),
        (
            CandidateSpec::Remote {
                url: "https://example.com:8443/path//".to_string(),
            },
            r#"["remote","https://example.com:8443/path"]"#,
            "3272b5ae1a1458980614ba56ceff0dfb5602122d7af7f1ec92440322cb042861",
        ),
        (
            CandidateSpec::Plugin {
                bundle_url: "https://plugins.example.com/bundle.zip".to_string(),
            },
            r#"["plugin","https://plugins.example.com/bundle.zip"]"#,
            "25f7444c18b703e83fd6fee628ef3bf8f0af73b76044fb37d0c247b2bd046611",
        ),
    ];
    for (spec, expected_canonical, expected_hash) in cases {
        assert_eq!(spec.canonical_string(), expected_canonical, "{spec:?}");
        assert_eq!(spec.canonical_hash(), expected_hash, "{spec:?}");
    }
}

#[test]
fn normalize_url_matches_identity_js() {
    assert_eq!(normalize_url("https://example.com/"), "https://example.com");
    assert_eq!(
        normalize_url("HTTPS://Example.COM:443/mcp/"),
        "https://example.com/mcp"
    );
    assert_eq!(
        normalize_url("https://example.com/mcp?b=2&a=1#frag"),
        "https://example.com/mcp?b=2&a=1"
    );
    assert_eq!(
        normalize_url("https://example.com:8443/path//"),
        "https://example.com:8443/path"
    );
    assert_eq!(normalize_url("  not a url at all  "), "not a url at all");
}

// ---------------------------------------------------------------------------
// Allowlist entry matching semantics.
// ---------------------------------------------------------------------------

#[test]
fn registry_name_entry_matches_exact_name_only() {
    let policy = allowlist_policy(vec![vec![allowlist_entry("github")]]);
    assert!(policy.allows_install(&stdio_candidate("github", "npx")));
    assert!(!policy.allows_install(&stdio_candidate("github-evil", "npx")));
    assert!(!policy.allows_install(&stdio_candidate("GitHub", "npx")));
}

#[test]
fn gallery_template_entry_matches_by_uuid() {
    let gallery_id = Uuid::new_v4();
    let policy = allowlist_policy(vec![vec![entry(
        McpAllowlistEntryKind::GalleryTemplate,
        &gallery_id.to_string(),
    )]]);

    let mut candidate = stdio_candidate("anything", "npx");
    candidate.gallery_template_id = Some(gallery_id);
    assert!(policy.allows_install(&candidate));

    candidate.gallery_template_id = Some(Uuid::new_v4());
    assert!(!policy.allows_install(&candidate));

    candidate.gallery_template_id = None;
    assert!(!policy.allows_install(&candidate));
}

#[test]
fn url_pattern_entry_prefix_and_wildcard_semantics() {
    // No `*` ⇒ prefix match on the normalized URL.
    let prefix = allowlist_policy(vec![vec![entry(
        McpAllowlistEntryKind::UrlPattern,
        "https://mcp.corp.example.com",
    )]]);
    assert!(prefix.allows_install(&remote_candidate(
        "corp",
        // Normalization lowercases the host and strips the default port.
        "https://MCP.corp.example.com:443/tools/search"
    )));
    assert!(!prefix.allows_install(&remote_candidate("other", "https://evil.example.com/mcp")));

    // `*` ⇒ wildcard match over the whole normalized URL.
    let wildcard = allowlist_policy(vec![vec![entry(
        McpAllowlistEntryKind::UrlPattern,
        "https://*.example.com/mcp",
    )]]);
    assert!(wildcard.allows_install(&remote_candidate("a", "https://a.example.com/mcp")));
    assert!(wildcard.allows_install(&remote_candidate("b", "https://b.c.example.com/mcp/")));
    assert!(!wildcard.allows_install(&remote_candidate("c", "https://a.example.com/other")));

    // URL patterns never match stdio candidates.
    assert!(!prefix.allows_install(&stdio_candidate("corp", "npx")));
}

#[test]
fn command_pattern_entry_exact_and_wildcard_semantics() {
    // No `*` ⇒ exact match (prefix matching would let `npx-evil` through).
    let exact = allowlist_policy(vec![vec![entry(
        McpAllowlistEntryKind::CommandPattern,
        "npx",
    )]]);
    assert!(exact.allows_install(&stdio_candidate("s", "npx")));
    assert!(!exact.allows_install(&stdio_candidate("s", "npx-evil")));

    // `*` ⇒ wildcard match over the command.
    let wildcard = allowlist_policy(vec![vec![entry(
        McpAllowlistEntryKind::CommandPattern,
        "/usr/local/bin/*",
    )]]);
    assert!(wildcard.allows_install(&stdio_candidate("s", "/usr/local/bin/mcp-server")));
    assert!(!wildcard.allows_install(&stdio_candidate("s", "/usr/bin/mcp-server")));

    // Command patterns never match remote candidates.
    assert!(!exact.allows_install(&remote_candidate("s", "https://example.com/mcp")));
}

#[test]
fn canonical_hash_entry_matches_case_insensitively() {
    let spec = CandidateSpec::Stdio {
        command: "uvx".to_string(),
        args: vec![],
        env_keys: vec![],
    };
    let hash = spec.canonical_hash();
    let candidate = ServerCandidate {
        name: Some("anything".to_string()),
        spec: Some(spec),
        ..Default::default()
    };

    let policy = allowlist_policy(vec![vec![entry(
        McpAllowlistEntryKind::CanonicalHash,
        &hash.to_uppercase(),
    )]]);
    assert!(policy.allows_install(&candidate));

    let wrong = allowlist_policy(vec![vec![entry(
        McpAllowlistEntryKind::CanonicalHash,
        &"0".repeat(64),
    )]]);
    assert!(!wrong.allows_install(&candidate));
}

#[test]
fn pinned_version_is_honored_where_version_is_known() {
    let mut pinned = allowlist_entry("github");
    pinned.pinned_version = Some("3".to_string());
    let policy = allowlist_policy(vec![vec![pinned]]);

    let mut candidate = stdio_candidate("github", "npx");
    // Unknown candidate version ⇒ the pin does not constrain.
    assert!(policy.allows_install(&candidate));

    candidate.version = Some("3".to_string());
    assert!(policy.allows_install(&candidate));

    candidate.version = Some("4".to_string());
    assert!(!policy.allows_install(&candidate));
}

#[test]
fn multi_workspace_allowlists_require_intersection() {
    // The candidate must be allowed by EVERY governed allowlist.
    let policy = allowlist_policy(vec![
        vec![allowlist_entry("shared"), allowlist_entry("only-in-a")],
        vec![allowlist_entry("shared"), allowlist_entry("only-in-b")],
    ]);
    assert!(policy.allows_install(&stdio_candidate("shared", "npx")));
    assert!(!policy.allows_install(&stdio_candidate("only-in-a", "npx")));
    assert!(!policy.allows_install(&stdio_candidate("only-in-b", "npx")));
}

#[test]
fn allowlist_spawn_semantics() {
    let policy = allowlist_policy(vec![vec![allowlist_entry("allowed")]]);

    // Matching candidate spawns; non-matching is blocked.
    assert!(policy.allows_spawn(
        ServerOrigin::Manual,
        Some(&stdio_candidate("allowed", "npx"))
    ));
    assert!(!policy.allows_spawn(
        ServerOrigin::Manual,
        Some(&stdio_candidate("blocked", "npx"))
    ));

    // No fingerprint available ⇒ only the coarse gates apply (backend stays
    // authoritative for unfingerprintable servers).
    assert!(policy.allows_spawn(ServerOrigin::Manual, None));

    // File-based servers must pass BOTH the file-based gate and the
    // allowlist.
    assert!(policy.allows_spawn(
        ServerOrigin::FileBased,
        Some(&stdio_candidate("allowed", "npx"))
    ));
    assert!(!policy.allows_spawn(
        ServerOrigin::FileBased,
        Some(&stdio_candidate("blocked", "npx"))
    ));
    let no_file_based = EffectiveMcpPolicy {
        allow_file_based_servers: false,
        ..policy
    };
    assert!(!no_file_based.allows_spawn(
        ServerOrigin::FileBased,
        Some(&stdio_candidate("allowed", "npx"))
    ));
}

#[test]
fn candidate_from_template_parses_stdio_and_remote_specs() {
    let mut stdio_template = TemplatableMCPServer {
        name: "files".to_string(),
        ..Default::default()
    };
    stdio_template.template.json = r#"{
        "files": {
            "command": "npx",
            "args": ["-y", "server"],
            "env": {"B_KEY": "x", "A_KEY": "y"}
        }
    }"#
    .to_string();
    let candidate = ServerCandidate::from_template(&stdio_template);
    assert_eq!(candidate.name.as_deref(), Some("files"));
    assert_eq!(
        candidate.spec,
        Some(CandidateSpec::Stdio {
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "server".to_string()],
            env_keys: vec!["A_KEY".to_string(), "B_KEY".to_string()],
        })
    );

    let mut remote_template = TemplatableMCPServer {
        name: "remote".to_string(),
        ..Default::default()
    };
    remote_template.template.json = r#"{
        "remote": { "url": "https://example.com/mcp" }
    }"#
    .to_string();
    let candidate = ServerCandidate::from_template(&remote_template);
    assert_eq!(
        candidate.spec,
        Some(CandidateSpec::Remote {
            url: "https://example.com/mcp".to_string()
        })
    );

    // Malformed template JSON ⇒ no spec, but name matching still works.
    let mut broken = TemplatableMCPServer {
        name: "broken".to_string(),
        ..Default::default()
    };
    broken.template.json = "not json".to_string();
    assert_eq!(ServerCandidate::from_template(&broken).spec, None);
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

    App::test((), |app| async move {
        let handle = app.add_singleton_model(move |ctx| McpGovernance::new(Some(cached_json), ctx));
        handle.read(&app, |governance, _| {
            assert_eq!(governance.effective_policy(), &cached);
        });
    });
}

#[test]
fn model_falls_back_to_self_managed_without_or_with_corrupt_cache() {
    App::test((), |app| async move {
        let no_cache = app.add_singleton_model(|ctx| McpGovernance::new(None, ctx));
        no_cache.read(&app, |governance, _| {
            assert_eq!(
                governance.effective_policy(),
                &EffectiveMcpPolicy::self_managed()
            );
        });
    });

    App::test((), |app| async move {
        let corrupt_cache =
            app.add_singleton_model(|ctx| McpGovernance::new(Some("not json".to_string()), ctx));
        corrupt_cache.read(&app, |governance, _| {
            assert_eq!(
                governance.effective_policy(),
                &EffectiveMcpPolicy::self_managed()
            );
        });
    });
}
