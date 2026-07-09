use warp_graphql::queries::search_marketplace::MarketplacePluginComponents;

use super::*;

fn components(
    mcp: i32,
    rules: i32,
    skills: i32,
    agents: i32,
    commands: i32,
    hooks: i32,
) -> MarketplacePluginComponents {
    MarketplacePluginComponents {
        agent_count: agents,
        command_count: commands,
        hook_count: hooks,
        mcp_server_count: mcp,
        rule_count: rules,
        skill_count: skills,
    }
}

fn entry(
    components: Option<MarketplacePluginComponents>,
    category: Option<&str>,
    install_count: Option<i32>,
) -> MarketplaceSearchEntry {
    MarketplaceSearchEntry {
        author: None,
        bundle_url: None,
        category: category.map(str::to_string),
        components,
        description: String::new(),
        entry_id: cynic::Id::new("id"),
        extension_id: None,
        homepage: None,
        icon_url: None,
        install_count,
        kind: MarketplaceEntryKind::Plugin,
        license: None,
        mcp_template_json: None,
        publisher: None,
        source_label: "Source".to_string(),
        tags: None,
        title: "Title".to_string(),
        version: None,
    }
}

#[test]
fn pluralize_singular_and_plural() {
    assert_eq!(pluralize(1, "rule"), "1 rule");
    assert_eq!(pluralize(0, "rule"), "0 rules");
    assert_eq!(pluralize(3, "skill"), "3 skills");
}

#[test]
fn sanitize_path_component_rejects_traversal() {
    assert_eq!(
        sanitize_path_component("my-skill").as_deref(),
        Some("my-skill")
    );
    assert!(sanitize_path_component("..").is_none());
    assert!(sanitize_path_component(".").is_none());
    assert!(sanitize_path_component("a/b").is_none());
    assert!(sanitize_path_component("a\\b").is_none());
    assert!(sanitize_path_component("   ").is_none());
}

#[test]
fn sanitize_relative_path_strips_and_guards() {
    assert_eq!(
        sanitize_relative_path("SKILL.md"),
        Some(std::path::PathBuf::from("SKILL.md"))
    );
    assert_eq!(
        sanitize_relative_path("dir/./nested/file.md"),
        Some(std::path::PathBuf::from("dir/nested/file.md"))
    );
    assert!(sanitize_relative_path("../escape").is_none());
    assert!(sanitize_relative_path("a/../b").is_none());
    assert!(sanitize_relative_path("").is_none());
}

#[test]
fn component_badges_lists_category_then_present_types() {
    let entry = entry(
        Some(components(2, 1, 0, 0, 3, 1)),
        Some("Productivity"),
        Some(5),
    );
    let badges = MarketplaceDirectoryView::component_badges(&entry);
    assert_eq!(
        badges,
        vec![
            "Productivity".to_string(),
            "MCP".to_string(),
            "Rules".to_string(),
            "Commands".to_string(),
            "Hooks".to_string(),
        ]
    );
}

#[test]
fn component_badges_empty_without_components_or_category() {
    let entry = entry(None, None, None);
    assert!(MarketplaceDirectoryView::component_badges(&entry).is_empty());
}

#[test]
fn prettify_category_title_cases_slugs() {
    assert_eq!(prettify_category("developer-tools"), "Developer Tools");
    assert_eq!(prettify_category("data_analytics"), "Data Analytics");
    assert_eq!(
        prettify_category("  agent orchestration "),
        "Agent Orchestration"
    );
    assert_eq!(prettify_category("utilities"), "Utilities");
    assert_eq!(prettify_category(""), "");
}

#[test]
fn is_cursor_plugin_requires_components_and_no_extension() {
    let cursor = entry(Some(components(1, 0, 0, 0, 0, 0)), None, None);
    assert!(MarketplaceDirectoryView::is_cursor_plugin(&cursor));

    let plain = entry(None, None, None);
    assert!(!MarketplaceDirectoryView::is_cursor_plugin(&plain));

    let mut vsx = entry(Some(components(1, 0, 0, 0, 0, 0)), None, None);
    vsx.extension_id = Some("pub.ext".to_string());
    assert!(!MarketplaceDirectoryView::is_cursor_plugin(&vsx));
}
