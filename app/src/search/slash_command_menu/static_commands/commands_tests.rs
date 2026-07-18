use std::collections::HashSet;

use super::*;

#[test]
fn command_names_are_unique() {
    let names = COMMAND_REGISTRY.all_commands().map(|command| command.name);
    let mut seen = HashSet::new();
    for name in names {
        assert!(seen.insert(name), "duplicate slash command name: {name}");
    }
}

#[test]
fn rename_tab_command_requires_argument() {
    let command = COMMAND_REGISTRY
        .get_command_with_name(RENAME_TAB.name)
        .expect("expected /rename-tab to be registered");
    let argument = command
        .argument
        .as_ref()
        .expect("expected /rename-tab to require an argument");

    assert!(!argument.is_optional);
    assert!(!argument.should_execute_on_selection);
    assert_eq!(argument.hint_text, Some("<tab name>"));
}

#[test]
fn rename_conversation_command_is_active_conversation_scoped_and_requires_argument() {
    let command = COMMAND_REGISTRY
        .get_command_with_name(RENAME_CONVERSATION.name)
        .expect("expected /rename-conversation to be registered");
    let argument = command
        .argument
        .as_ref()
        .expect("expected /rename-conversation to require an argument");

    assert_eq!(command.name, "/rename-conversation");
    assert_eq!(command.icon_path, "bundled/svg/pencil-line.svg");
    assert!(!command.auto_enter_ai_mode);
    assert_eq!(
        command.availability,
        Availability::AGENT_VIEW | Availability::ACTIVE_CONVERSATION | Availability::AI_ENABLED,
    );
    assert!(!argument.is_optional);
    assert!(!argument.should_execute_on_selection);
    assert_eq!(argument.hint_text, Some("<new title>"));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn continue_locally_command_is_registered() {
    let command = COMMAND_REGISTRY
        .get_command_with_name(CONTINUE_LOCALLY.name)
        .expect("expected /continue-locally to be registered");

    assert_eq!(command.name, "/continue-locally");
    assert_eq!(command.icon_path, "bundled/svg/arrow-split.svg");
    assert!(command.auto_enter_ai_mode);
    assert_eq!(
        command.availability,
        Availability::AGENT_VIEW
            | Availability::ACTIVE_CONVERSATION
            | Availability::AI_ENABLED
            | Availability::CLOUD_AGENT
    );

    let argument = command
        .argument
        .as_ref()
        .expect("expected /continue-locally to declare an argument");
    assert!(argument.is_optional);
    assert!(!argument.should_execute_on_selection);
    assert_eq!(
        argument.hint_text,
        Some("<optional prompt to send in local conversation>")
    );
}

#[test]
fn set_tab_color_command_requires_argument() {
    let command = COMMAND_REGISTRY
        .get_command_with_name(SET_TAB_COLOR.name)
        .expect("expected /set-tab-color to be registered");
    let argument = command
        .argument
        .as_ref()
        .expect("expected /set-tab-color to require an argument");

    assert!(!argument.is_optional);
    assert!(!argument.should_execute_on_selection);

    let hint = argument
        .hint_text
        .expect("/set-tab-color hint text is set dynamically");
    for color in color_dot::TAB_COLOR_OPTIONS {
        let lower = color.to_string().to_ascii_lowercase();
        assert!(hint.contains(&lower), "hint should mention `{lower}`");
    }
    assert!(hint.contains("none"), "hint should mention `none`");
}

#[test]
fn strip_command_prefix_matches_orchestrate() {
    let result = strip_command_prefix("/orchestrate deploy services", "/orchestrate");
    assert_eq!(result, Some("deploy services".to_string()));
}

#[test]
fn multitask_is_registered_as_orchestrate_alias() {
    let multitask = COMMAND_REGISTRY
        .get_command_with_name(MULTITASK_NAME)
        .expect("expected /multitask to be registered");

    // /multitask is an alias for /orchestrate: identical availability, icon, and
    // AI-mode behavior so both route through the same orchestrate query mode.
    assert_eq!(multitask.name, "/multitask");
    assert_eq!(multitask.icon_path, ORCHESTRATE.icon_path);
    assert_eq!(multitask.availability, ORCHESTRATE.availability);
    assert_eq!(multitask.auto_enter_ai_mode, ORCHESTRATE.auto_enter_ai_mode);
    assert!(multitask.argument.as_ref().is_some_and(|a| a.is_optional));
}

#[test]
fn strip_command_prefix_matches_multitask() {
    let result = strip_command_prefix("/multitask deploy services", "/multitask");
    assert_eq!(result, Some("deploy services".to_string()));
}

#[test]
fn steer_command_sends_now_and_requires_argument() {
    // /steer is the mid-run sibling of /queue: same conversation scoping, but it
    // delivers the message to the running agent immediately (via the mailbox)
    // rather than queuing it for after the turn. Registration is feature-gated
    // (SteerSlashCommand), so assert the static definition directly.
    assert_eq!(STEER.name, "/steer");
    assert_eq!(STEER.name, STEER_NAME);
    assert_eq!(STEER.icon_path, "bundled/svg/navigation.svg");
    assert!(STEER.auto_enter_ai_mode);
    assert_eq!(STEER.availability, QUEUE.availability);

    let argument = STEER
        .argument
        .as_ref()
        .expect("expected /steer to require an argument");
    assert!(!argument.is_optional);
}

#[test]
fn strip_command_prefix_no_match() {
    let result = strip_command_prefix("just a normal query", "/plan");
    assert_eq!(result, None);
}

#[test]
fn strip_command_prefix_empty() {
    let result = strip_command_prefix("", "/plan");
    assert_eq!(result, None);
}

#[test]
fn strip_command_prefix_no_trailing_space() {
    // "/plan" alone (no trailing space) should NOT be stripped
    let result = strip_command_prefix("/plan", "/plan");
    assert_eq!(result, None);
}

#[test]
fn strip_command_prefix_trailing_space_only() {
    // "/plan " with nothing after should strip to empty string
    let result = strip_command_prefix("/plan ", "/plan");
    assert_eq!(result, Some(String::new()));
}

#[test]
fn strip_command_prefix_substring_not_matched() {
    // "/planning" should not match "/plan"
    let result = strip_command_prefix("/planning something", "/plan");
    assert_eq!(result, None);
}
