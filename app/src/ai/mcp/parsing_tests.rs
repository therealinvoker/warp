#[cfg(feature = "local_fs")]
use std::path::Path;

#[cfg(feature = "local_fs")]
use super::normalize_cursor_json;
use super::ParsedTemplatableMCPServerResult;
use crate::ai::mcp::ServerOrigin;

#[test]
fn config_file_json_ignores_unrelated_settings() {
    // ~/.claude.json contains Claude Code app settings, not MCP servers.
    let claude_code_settings = r#"{
        "numStartups": 37,
        "tipsHistory": { "new-user-warmup": 9 },
        "projects": {},
        "officialMarketplaceAutoInstallAttempted": true,
        "sonnet45MigrationComplete": true
    }"#;

    let servers = ParsedTemplatableMCPServerResult::from_config_file_json(claude_code_settings)
        .expect("valid JSON should not error");
    assert!(
        servers.is_empty(),
        "Claude Code settings should not be parsed as MCP servers"
    );
}

#[test]
fn config_file_json_parses_mcp_servers_key() {
    let json = r#"{
        "mcpServers": {
            "github": {
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-github"]
            }
        }
    }"#;

    let servers = ParsedTemplatableMCPServerResult::from_config_file_json(json)
        .expect("valid JSON should not error");
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].templatable_mcp_server.name, "github");
}

#[test]
fn config_file_json_parses_mcp_dot_servers_key() {
    let json = r#"{
        "mcp": {
            "servers": {
                "my-server": { "command": "uvx", "args": ["mcp-server"] }
            }
        }
    }"#;

    let servers = ParsedTemplatableMCPServerResult::from_config_file_json(json)
        .expect("valid JSON should not error");
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].templatable_mcp_server.name, "my-server");
}

#[test]
fn config_file_json_parses_mcp_underscore_servers_key() {
    let json = r#"{
        "mcp_servers": {
            "s": { "url": "https://example.com/mcp" }
        }
    }"#;

    let servers = ParsedTemplatableMCPServerResult::from_config_file_json(json)
        .expect("valid JSON should not error");
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].templatable_mcp_server.name, "s");
}

#[test]
fn config_file_json_returns_error_for_invalid_json() {
    let result = ParsedTemplatableMCPServerResult::from_config_file_json("not json");
    assert!(result.is_err());
}

#[test]
fn from_user_json_still_accepts_bare_server_map() {
    // The permissive from_user_json should continue to accept bare maps
    // (for UI paste scenarios).
    let json = r#"{
        "github": {
            "command": "npx",
            "args": ["-y", "@modelcontextprotocol/server-github"]
        }
    }"#;

    let servers =
        ParsedTemplatableMCPServerResult::from_user_json(json).expect("should parse bare map");
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].templatable_mcp_server.name, "github");
}

// ── ServerOrigin provenance ────────────────────────────────────────────

#[test]
fn config_file_json_marks_results_as_file_based() {
    let json = r#"{ "mcpServers": { "s": { "command": "npx" } } }"#;
    let servers = ParsedTemplatableMCPServerResult::from_config_file_json(json)
        .expect("valid JSON should not error");
    assert_eq!(
        servers[0].templatable_mcp_server.origin,
        ServerOrigin::FileBased
    );
    assert_eq!(
        servers[0]
            .templatable_mcp_server_installation
            .as_ref()
            .expect("installation should be present")
            .origin(),
        ServerOrigin::FileBased
    );
}

#[test]
fn from_user_json_results_default_to_manual_origin() {
    let json = r#"{ "s": { "command": "npx" } }"#;
    let servers =
        ParsedTemplatableMCPServerResult::from_user_json(json).expect("should parse bare map");
    assert_eq!(
        servers[0].templatable_mcp_server.origin,
        ServerOrigin::Manual
    );
}

#[test]
fn with_origin_updates_template_and_installation() {
    let json = r#"{ "mcpServers": { "s": { "command": "npx" } } }"#;
    let servers = ParsedTemplatableMCPServerResult::from_config_file_json(json)
        .expect("valid JSON should not error");
    let result = servers[0].clone().with_origin(ServerOrigin::CursorImport);
    assert_eq!(
        result.templatable_mcp_server.origin,
        ServerOrigin::CursorImport
    );
    assert_eq!(
        result
            .templatable_mcp_server_installation
            .as_ref()
            .expect("installation should be present")
            .origin(),
        ServerOrigin::CursorImport
    );
}

// ── Cursor mcp.json normalizer ─────────────────────────────────────────

/// Normalizes and returns the parsed output value for easy assertions.
#[cfg(feature = "local_fs")]
fn normalize_to_value(input: &str, workspace_root: Option<&Path>) -> serde_json::Value {
    let json = normalize_cursor_json(input, workspace_root).expect("normalization should succeed");
    serde_json::from_str(&json).expect("normalized output should be valid JSON")
}

/// Servers are read from the `mcpServers` wrapper and unknown top-level keys
/// (e.g. Cursor's `__playbook_managed`) are ignored.
#[cfg(feature = "local_fs")]
#[test]
fn cursor_json_reads_mcp_servers_wrapper_and_ignores_stray_top_level_keys() {
    let input = r#"{
        "__playbook_managed": true,
        "somethingElse": { "command": "should-not-appear" },
        "mcpServers": {
            "github": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-github"] }
        }
    }"#;
    let value = normalize_to_value(input, None);
    let servers = value["mcpServers"].as_object().expect("object");
    assert_eq!(
        servers.len(),
        1,
        "only servers under mcpServers should be read"
    );
    assert_eq!(
        value["mcpServers"]["github"]["command"].as_str(),
        Some("npx")
    );
}

/// The Cursor `type` tag is stripped for stdio servers.
#[cfg(feature = "local_fs")]
#[test]
fn cursor_json_strips_stdio_type_tag() {
    let input = r#"{
        "mcpServers": {
            "s": { "type": "stdio", "command": "npx", "args": ["-y", "x"] }
        }
    }"#;
    let value = normalize_to_value(input, None);
    assert!(
        value["mcpServers"]["s"].get("type").is_none(),
        "type tag should be stripped"
    );
    assert_eq!(value["mcpServers"]["s"]["command"].as_str(), Some("npx"));
}

/// Url-typed servers (sse / http / streamable-http) map onto the url-based
/// config with the type tag stripped and headers preserved.
#[cfg(feature = "local_fs")]
#[test]
fn cursor_json_maps_url_typed_servers_to_url_config() {
    for server_type in ["sse", "http", "streamable-http"] {
        let input = format!(
            r#"{{
                "mcpServers": {{
                    "remote": {{
                        "type": "{server_type}",
                        "url": "https://example.com/mcp",
                        "headers": {{ "X-Api-Key": "abc" }}
                    }}
                }}
            }}"#
        );
        let value = normalize_to_value(&input, None);
        let server = &value["mcpServers"]["remote"];
        assert!(server.get("type").is_none(), "type should be stripped");
        assert_eq!(server["url"].as_str(), Some("https://example.com/mcp"));
        assert_eq!(server["headers"]["X-Api-Key"].as_str(), Some("abc"));
    }
}

/// `${env:VAR}` interpolations become `${VAR}` placeholders (the same form
/// Codex `env_vars` are lowered to), wherever they appear.
#[cfg(feature = "local_fs")]
#[test]
fn cursor_json_rewrites_env_interpolations() {
    let input = r#"{
        "mcpServers": {
            "s": {
                "command": "npx",
                "args": ["--token", "${env:MY_TOKEN}"],
                "env": { "API_KEY": "${env:API_KEY}" }
            }
        }
    }"#;
    let value = normalize_to_value(input, None);
    let server = &value["mcpServers"]["s"];
    assert_eq!(server["env"]["API_KEY"].as_str(), Some("${API_KEY}"));
    assert_eq!(server["args"][1].as_str(), Some("${MY_TOKEN}"));
}

/// `${workspaceFolder}` is replaced with the workspace root when known, and
/// left untouched when the root is unknown.
#[cfg(feature = "local_fs")]
#[test]
fn cursor_json_replaces_workspace_folder_placeholder() {
    let input = r#"{
        "mcpServers": {
            "s": {
                "command": "node",
                "args": ["${workspaceFolder}/tools/server.js"],
                "cwd": "${workspaceFolder}"
            }
        }
    }"#;

    let value = normalize_to_value(input, Some(Path::new("/home/user/project")));
    let server = &value["mcpServers"]["s"];
    assert_eq!(
        server["args"][0].as_str(),
        Some("/home/user/project/tools/server.js")
    );
    assert_eq!(
        server["working_directory"].as_str(),
        Some("/home/user/project"),
        "cwd should map to working_directory with the placeholder resolved"
    );

    let value = normalize_to_value(input, None);
    assert_eq!(
        value["mcpServers"]["s"]["args"][0].as_str(),
        Some("${workspaceFolder}/tools/server.js"),
        "placeholder should be left untouched when the root is unknown"
    );
}

/// Cursor's `cwd` maps to Warp's `working_directory`.
#[cfg(feature = "local_fs")]
#[test]
fn cursor_json_maps_cwd_to_working_directory() {
    let input = r#"{
        "mcpServers": {
            "s": { "command": "npx", "cwd": "/some/dir" }
        }
    }"#;
    let value = normalize_to_value(input, None);
    let server = &value["mcpServers"]["s"];
    assert_eq!(server["working_directory"].as_str(), Some("/some/dir"));
    assert!(server.get("cwd").is_none(), "cwd key should be removed");
}

/// The `auth` block is dropped: Warp's MCP OAuth negotiates authorization at
/// connect time.
#[cfg(feature = "local_fs")]
#[test]
fn cursor_json_drops_auth_block() {
    let input = r#"{
        "mcpServers": {
            "remote": {
                "url": "https://example.com/mcp",
                "auth": { "provider": "cursor", "token": "should-never-survive" }
            }
        }
    }"#;
    let json = normalize_cursor_json(input, None).expect("normalization should succeed");
    assert!(
        !json.contains("should-never-survive"),
        "auth contents must not survive normalization"
    );
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert!(value["mcpServers"]["remote"].get("auth").is_none());
}

/// `envFile` is ignored without reading the file.
#[cfg(feature = "local_fs")]
#[test]
fn cursor_json_ignores_env_file_key() {
    let input = r#"{
        "mcpServers": {
            "s": { "command": "npx", "envFile": ".env.production" }
        }
    }"#;
    let value = normalize_to_value(input, None);
    assert!(
        value["mcpServers"]["s"].get("envFile").is_none(),
        "envFile should be dropped without being read"
    );
}

/// Malformed JSON is an error.
#[cfg(feature = "local_fs")]
#[test]
fn cursor_json_errors_on_malformed_json() {
    assert!(normalize_cursor_json("not json", None).is_err());
    assert!(normalize_cursor_json(r#"{"mcpServers": "#, None).is_err());
}

/// Non-object server entries are skipped without failing the whole file.
#[cfg(feature = "local_fs")]
#[test]
fn cursor_json_skips_malformed_entries() {
    let input = r#"{
        "mcpServers": {
            "bad": "not-an-object",
            "good": { "command": "npx" }
        }
    }"#;
    let value = normalize_to_value(input, None);
    let servers = value["mcpServers"].as_object().expect("object");
    assert_eq!(servers.len(), 1);
    assert!(servers.contains_key("good"));
}

/// A file without an `mcpServers` key normalizes to an empty server map.
#[cfg(feature = "local_fs")]
#[test]
fn cursor_json_without_mcp_servers_key_yields_empty_map() {
    let value = normalize_to_value(r#"{ "__playbook_managed": true }"#, None);
    assert_eq!(
        value["mcpServers"].as_object().map(|o| o.len()),
        Some(0),
        "no servers should be produced"
    );
}

/// Normalized Cursor output round-trips through the config-file parse path,
/// producing templatized env variables and masked installation values.
#[cfg(feature = "local_fs")]
#[test]
fn cursor_json_round_trips_through_config_file_parse() {
    let input = r#"{
        "mcpServers": {
            "github": {
                "type": "stdio",
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-github"],
                "env": { "GITHUB_TOKEN": "secret-value" }
            }
        }
    }"#;
    let json = normalize_cursor_json(input, None).expect("normalization should succeed");
    let parsed = ParsedTemplatableMCPServerResult::from_config_file_json(&json)
        .expect("config parse should succeed");
    assert_eq!(parsed.len(), 1);
    let result = &parsed[0];
    assert_eq!(result.templatable_mcp_server.name, "github");
    // Env value must be templatized out of the stored template JSON.
    assert!(
        !result
            .templatable_mcp_server
            .template
            .json
            .contains("secret-value"),
        "template JSON should not contain the raw env value"
    );
    assert_eq!(
        result
            .templatable_mcp_server
            .template
            .variables
            .iter()
            .map(|v| v.key.as_str())
            .collect::<Vec<_>>(),
        vec!["GITHUB_TOKEN"]
    );
    assert_eq!(
        result.variable_values["GITHUB_TOKEN"].value.as_str(),
        "secret-value"
    );
}
