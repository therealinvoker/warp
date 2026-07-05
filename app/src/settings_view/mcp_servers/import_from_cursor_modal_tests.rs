use super::{is_already_installed, summary_for_template};
use crate::ai::mcp::{ParsedTemplatableMCPServerResult, TemplatableMCPServer};

/// Builds a template through the same config-file parse path used by the
/// import scan.
fn template_from_config(config_json: &str) -> TemplatableMCPServer {
    let mut servers = ParsedTemplatableMCPServerResult::from_config_file_json(config_json)
        .expect("valid config JSON");
    assert_eq!(servers.len(), 1, "expected exactly one server in fixture");
    servers.remove(0).templatable_mcp_server
}

#[test]
fn summary_shows_command_and_args_for_stdio_servers() {
    let template = template_from_config(
        r#"{ "mcpServers": { "github": {
            "command": "npx",
            "args": ["-y", "@modelcontextprotocol/server-github"]
        } } }"#,
    );
    assert_eq!(
        summary_for_template(&template).as_deref(),
        Some("npx -y @modelcontextprotocol/server-github")
    );
}

#[test]
fn summary_shows_bare_command_when_no_args() {
    let template = template_from_config(r#"{ "mcpServers": { "s": { "command": "uvx" } } }"#);
    assert_eq!(summary_for_template(&template).as_deref(), Some("uvx"));
}

#[test]
fn summary_shows_url_for_remote_servers() {
    let template = template_from_config(
        r#"{ "mcpServers": { "remote": { "url": "https://example.com/mcp" } } }"#,
    );
    assert_eq!(
        summary_for_template(&template).as_deref(),
        Some("https://example.com/mcp")
    );
}

#[test]
fn summary_is_none_for_unrecognized_transport() {
    let template = template_from_config(r#"{ "mcpServers": { "odd": { "foo": "bar" } } }"#);
    assert_eq!(summary_for_template(&template), None);
}

#[test]
fn already_installed_matches_by_name_case_insensitively() {
    let existing = vec![(
        "GitHub".to_string(),
        r#"{"GitHub":{"command":"npx"}}"#.to_string(),
    )];
    assert!(is_already_installed(
        "github",
        r#"{"github":{"command":"something-else"}}"#,
        &existing
    ));
}

#[test]
fn already_installed_matches_by_structural_template_equality() {
    // Same template content, different formatting and key order.
    let existing = vec![(
        "other-name".to_string(),
        r#"{"srv": {"args": ["-y"], "command": "npx"}}"#.to_string(),
    )];
    assert!(is_already_installed(
        "srv-imported",
        r#"{"srv":{"command":"npx","args":["-y"]}}"#,
        &existing
    ));
}

#[test]
fn already_installed_is_false_for_new_servers() {
    let existing = vec![(
        "other".to_string(),
        r#"{"other":{"command":"uvx"}}"#.to_string(),
    )];
    assert!(!is_already_installed(
        "new-server",
        r#"{"new-server":{"command":"npx"}}"#,
        &existing
    ));
}

#[test]
fn already_installed_is_false_when_nothing_is_installed() {
    assert!(!is_already_installed(
        "server",
        r#"{"server":{"command":"npx"}}"#,
        &[]
    ));
}
