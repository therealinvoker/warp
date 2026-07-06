//! Inline create/edit form state for a GitHub automation.
//!
//! The list page ([`super::list_page`]) owns the text-editor handles (name,
//! repo filter, branch pattern, comment phrase, prompt, skill, model) and hosts
//! this form inline. [`AutomationFormState`] holds the non-text selections
//! (trigger type, action type, enabled) plus the id of the automation being
//! edited (or `None` when creating), and knows how to validate the assembled
//! values and build a [`GithubAutomationInput`].

use crate::github::automations::{
    GithubAutomation, GithubAutomationAction, GithubAutomationActionType, GithubAutomationInput,
    GithubAutomationTrigger, GithubAutomationTriggerType,
};

/// Non-text form state for the automation editor.
#[derive(Debug, Clone)]
pub struct AutomationFormState {
    /// `Some` when editing an existing automation; `None` when creating.
    pub id: Option<String>,
    pub enabled: bool,
    pub trigger_type: GithubAutomationTriggerType,
    pub action_type: GithubAutomationActionType,
}

impl Default for AutomationFormState {
    fn default() -> Self {
        Self {
            id: None,
            enabled: true,
            trigger_type: GithubAutomationTriggerType::PrOpened,
            action_type: GithubAutomationActionType::Prompt,
        }
    }
}

impl AutomationFormState {
    /// Seed the form from an existing automation for editing.
    pub fn from_automation(automation: &GithubAutomation) -> Self {
        Self {
            id: Some(automation.id.clone()),
            enabled: automation.enabled,
            trigger_type: automation.trigger.event_type,
            action_type: automation.action.action_type,
        }
    }

    pub fn is_editing(&self) -> bool {
        self.id.is_some()
    }
}

/// The assembled, trimmed text values from the editors, paired with the
/// [`AutomationFormState`] selections.
pub struct AutomationFormValues<'a> {
    pub state: &'a AutomationFormState,
    pub name: String,
    pub repo_filter: String,
    pub branch_pattern: String,
    pub comment_phrase: String,
    pub prompt: String,
    pub skill: String,
    pub harness: String,
    pub model_id: String,
}

impl AutomationFormValues<'_> {
    /// Validate the form, returning a user-facing error string on failure.
    ///
    /// `repo_allowlist` is the workspace repo allowlist (installed repos); when
    /// non-empty, a set `repo_filter` must intersect it. An empty allowlist means
    /// "not restricted" and any filter is accepted.
    pub fn validate(&self, repo_allowlist: &[String]) -> Result<GithubAutomationInput, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("Name is required.".to_string());
        }

        let repo_filter = non_empty(&self.repo_filter);
        if let Some(filter) = repo_filter.as_deref() {
            if !repo_allowlist.is_empty()
                && !repo_allowlist
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(filter))
            {
                return Err(format!(
                    "Repo filter \"{filter}\" is not in this workspace's allowed repositories."
                ));
            }
        }

        if self.state.trigger_type.is_custom() && non_empty(&self.comment_phrase).is_some() {
            // Comment phrase only applies to comment-driven triggers; ignore it
            // for custom webhooks rather than error (kept lenient).
        }

        let action = match self.state.action_type {
            GithubAutomationActionType::Prompt => {
                let prompt = self.prompt.trim();
                if prompt.is_empty() {
                    return Err("A prompt is required for the Prompt action.".to_string());
                }
                GithubAutomationAction {
                    action_type: GithubAutomationActionType::Prompt,
                    prompt: Some(prompt.to_string()),
                    skill: None,
                    harness: non_empty(&self.harness),
                    model_id: non_empty(&self.model_id),
                }
            }
            GithubAutomationActionType::Skill => {
                let skill = self.skill.trim();
                if skill.is_empty() {
                    return Err("A skill name is required for the Skill action.".to_string());
                }
                GithubAutomationAction {
                    action_type: GithubAutomationActionType::Skill,
                    prompt: None,
                    skill: Some(skill.to_string()),
                    harness: non_empty(&self.harness),
                    model_id: non_empty(&self.model_id),
                }
            }
        };

        let trigger = GithubAutomationTrigger {
            event_type: self.state.trigger_type,
            repo_filter,
            branch_pattern: non_empty(&self.branch_pattern),
            comment_phrase: non_empty(&self.comment_phrase),
        };

        Ok(GithubAutomationInput {
            id: self.state.id.clone(),
            name: name.to_string(),
            enabled: self.state.enabled,
            trigger,
            action,
        })
    }
}

/// Trim `s`; return `None` if empty after trimming.
fn non_empty(s: &str) -> Option<String> {
    let trimmed = s.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
#[path = "edit_page_tests.rs"]
mod tests;
