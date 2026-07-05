use super::{
    CLIServer, MCPServer, ServerOrigin, ServerSentEvents, StaticEnvVar, TemplatableMCPServer,
    TransportType,
};

#[test]
fn test_mcp_server_config_serialization_excludes_secret_env_values() {
    // Create a CLI server with environment variables containing secrets
    let cli_server = CLIServer {
        command: "npx".to_string(),
        args: vec!["@modelcontextprotocol/server-postgres".to_string()],
        cwd_parameter: Some("/tmp".to_string()),
        static_env_vars: vec![
            StaticEnvVar {
                name: "API_KEY".to_string(),
                value: "SOME_LEAKED_SECRET".to_string(),
            },
            StaticEnvVar {
                name: "DATABASE_URL".to_string(),
                value: "postgresql://user:password@localhost/db".to_string(),
            },
            StaticEnvVar {
                name: "PUBLIC_CONFIG".to_string(),
                value: "not-secret-value".to_string(),
            },
        ],
    };

    let mcp_server = MCPServer {
        transport_type: TransportType::CLIServer(cli_server),
        name: "test-server".to_string(),
        uuid: uuid::Uuid::new_v4(),
    };
    // Test direct serde serialization
    let serialized = serde_json::to_string(&mcp_server).expect("Failed to serialize MCP server");
    // The serialized config should NOT contain the secret values
    assert!(
        !serialized.contains("SOME_LEAKED_SECRET"),
        "Serialized config contains leaked secret value: {serialized}",
    );
    assert!(
        !serialized.contains("password"),
        "Serialized config contains password: {serialized}",
    );
    assert!(
        !serialized.contains("not-secret-value"),
        "Serialized config contains env var value: {serialized}",
    );
    // But should contain the environment variable names/keys
    assert!(
        serialized.contains("API_KEY"),
        "Serialized config should contain env var key 'API_KEY': {serialized}",
    );
    assert!(
        serialized.contains("DATABASE_URL"),
        "Serialized config should contain env var key 'DATABASE_URL': {serialized}",
    );
    assert!(
        serialized.contains("PUBLIC_CONFIG"),
        "Serialized config should contain env var key 'PUBLIC_CONFIG': {serialized}",
    );
}

#[test]
fn test_static_env_var_direct_serialization() {
    // Test direct serialization of StaticEnvVar to ensure skip_serializing works
    let env_var = StaticEnvVar {
        name: "TEST_SECRET".to_string(),
        value: "SOME_LEAKED_SECRET".to_string(),
    };

    let serialized = serde_json::to_string(&env_var).expect("Failed to serialize env var");

    // Should contain the name but not the value due to skip_serializing
    assert!(
        serialized.contains("TEST_SECRET"),
        "Serialized env var should contain name: {serialized}",
    );
    assert!(
        !serialized.contains("SOME_LEAKED_SECRET"),
        "Serialized env var should not contain value due to skip_serializing: {serialized}",
    );
}

#[test]
fn test_static_env_var_deserialization_with_default() {
    // Test that StaticEnvVar can be deserialized properly with default value
    let json = r#"{"name": "API_KEY"}"#;

    let env_var: StaticEnvVar = serde_json::from_str(json).expect("Failed to deserialize env var");

    assert_eq!(env_var.name, "API_KEY");
    assert_eq!(env_var.value, ""); // Should default to empty string
}

#[test]
fn test_sse_server_serialization() {
    // Test that ServerSentEvents transport type serializes correctly
    let sse_server = ServerSentEvents {
        url: "https://example.com/sse".to_string(),
        headers: Default::default(),
    };

    let mcp_server = MCPServer {
        transport_type: TransportType::ServerSentEvents(sse_server),
        name: "sse-server".to_string(),
        uuid: uuid::Uuid::new_v4(),
    };

    let serialized = serde_json::to_string(&mcp_server).expect("Failed to serialize MCP server");

    // Should contain the URL since it's not a secret field
    assert!(
        serialized.contains("https://example.com/sse"),
        "Serialized SSE server should contain URL: {serialized}",
    );
    assert!(
        serialized.contains("sse-server"),
        "Serialized SSE server should contain name: {serialized}",
    );
}

// ── ServerOrigin provenance ────────────────────────────────────────────

/// Templates serialized before `origin` existed must deserialize with the
/// `Manual` default. This is what lets provenance ride the existing
/// installation/model JSON blobs without a schema migration.
#[test]
fn test_templatable_mcp_server_origin_defaults_to_manual_when_absent() {
    let json = r#"{
        "uuid": "6e5cbe0e-8c3c-4b0d-a26d-0f6a5c2a2c1e",
        "name": "legacy-server",
        "description": null,
        "template": { "json": "{}", "variables": [] },
        "version": 1,
        "gallery_data": null
    }"#;

    let server: TemplatableMCPServer =
        serde_json::from_str(json).expect("legacy JSON without origin should deserialize");
    assert_eq!(server.origin, ServerOrigin::Manual);
}

/// `origin` round-trips through serde for every variant.
#[test]
fn test_templatable_mcp_server_origin_round_trips() {
    for origin in [
        ServerOrigin::Manual,
        ServerOrigin::Gallery,
        ServerOrigin::CursorImport,
        ServerOrigin::Registry,
        ServerOrigin::OrgMarketplace,
        ServerOrigin::FileBased,
    ] {
        let server = TemplatableMCPServer {
            origin,
            ..Default::default()
        };
        let serialized = serde_json::to_string(&server).expect("serialization should succeed");
        let deserialized: TemplatableMCPServer =
            serde_json::from_str(&serialized).expect("deserialization should succeed");
        assert_eq!(deserialized.origin, origin, "origin should round-trip");
    }
}

/// The `Default` impl (used for structs constructed without an explicit
/// origin) is `Manual`.
#[test]
fn test_server_origin_default_is_manual() {
    assert_eq!(ServerOrigin::default(), ServerOrigin::Manual);
    assert_eq!(TemplatableMCPServer::default().origin, ServerOrigin::Manual);
}

/// `from_user_json` (paste/import parse path in the shared model crate)
/// produces Manual-origin templates.
#[test]
fn test_from_user_json_defaults_origin_to_manual() {
    let json = r#"{ "s": { "command": "npx" } }"#;
    let servers = TemplatableMCPServer::from_user_json(json).expect("should parse");
    assert_eq!(servers[0].origin, ServerOrigin::Manual);
}
