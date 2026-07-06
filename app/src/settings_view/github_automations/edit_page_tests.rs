use super::*;
use crate::github::automations::{GithubAutomationActionType, GithubAutomationTriggerType};

fn values<'a>(state: &'a AutomationFormState, name: &str, prompt: &str) -> AutomationFormValues<'a> {
    AutomationFormValues {
        state,
        name: name.to_string(),
        repo_filter: String::new(),
        branch_pattern: String::new(),
        comment_phrase: String::new(),
        prompt: prompt.to_string(),
        skill: String::new(),
        harness: String::new(),
        model_id: String::new(),
    }
}

#[test]
fn empty_name_is_rejected() {
    let state = AutomationFormState::default();
    let v = values(&state, "   ", "do the thing");
    assert!(v.validate(&[]).is_err());
}

#[test]
fn prompt_action_requires_prompt() {
    let state = AutomationFormState::default();
    let v = values(&state, "My automation", "  ");
    let err = v.validate(&[]).unwrap_err();
    assert!(err.to_lowercase().contains("prompt"));
}

#[test]
fn valid_prompt_automation_builds_input() {
    let state = AutomationFormState::default();
    let v = values(&state, "My automation", "review this PR");
    let input = v.validate(&[]).unwrap();
    assert_eq!(input.name, "My automation");
    assert!(input.enabled);
    assert_eq!(input.trigger.event_type, GithubAutomationTriggerType::PrOpened);
    assert_eq!(input.action.action_type, GithubAutomationActionType::Prompt);
    assert_eq!(input.action.prompt.as_deref(), Some("review this PR"));
    assert!(input.id.is_none());
}

#[test]
fn skill_action_requires_skill() {
    let state = AutomationFormState {
        action_type: GithubAutomationActionType::Skill,
        ..AutomationFormState::default()
    };
    let v = values(&state, "Skill automation", "");
    assert!(v.validate(&[]).is_err());
}

#[test]
fn repo_filter_must_intersect_non_empty_allowlist() {
    let state = AutomationFormState::default();
    let mut v = values(&state, "My automation", "do it");
    v.repo_filter = "octo/private".to_string();
    // Not in allowlist -> rejected.
    let allowlist = vec!["octo/public".to_string()];
    assert!(v.validate(&allowlist).is_err());
    // In allowlist (case-insensitive) -> accepted.
    let allowlist = vec!["OCTO/PRIVATE".to_string()];
    assert!(v.validate(&allowlist).is_ok());
}

#[test]
fn empty_allowlist_allows_any_repo_filter() {
    let state = AutomationFormState::default();
    let mut v = values(&state, "My automation", "do it");
    v.repo_filter = "anyone/anything".to_string();
    assert!(v.validate(&[]).is_ok());
}

#[test]
fn editing_preserves_id() {
    let state = AutomationFormState {
        id: Some("auto-123".to_string()),
        ..AutomationFormState::default()
    };
    let v = values(&state, "Edited", "prompt");
    let input = v.validate(&[]).unwrap();
    assert_eq!(input.id.as_deref(), Some("auto-123"));
}
