//! Plain (non-cynic) domain types for GitHub automations and workspace provider
//! keys, plus conversions from the generated GraphQL fragment types.
//!
//! The UI (settings pages) works with these types so it never touches cynic
//! types directly. Conversions live here to keep the server-api client thin.
//!
//! Gated on [`crate::features::FeatureFlag::GithubAutomations`] at the call
//! sites; this module is only compiled with the `github_automations` feature.

use chrono::{DateTime, Utc};

/// The kind of event that triggers a GitHub automation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubAutomationTriggerType {
    PrOpened,
    PrPushed,
    PrMerged,
    IssueComment,
    PrReviewSubmitted,
    WorkflowRunCompleted,
    Custom,
}

impl GithubAutomationTriggerType {
    /// All selectable trigger types, in menu order.
    pub fn all() -> &'static [GithubAutomationTriggerType] {
        &[
            GithubAutomationTriggerType::PrOpened,
            GithubAutomationTriggerType::PrPushed,
            GithubAutomationTriggerType::PrMerged,
            GithubAutomationTriggerType::IssueComment,
            GithubAutomationTriggerType::PrReviewSubmitted,
            GithubAutomationTriggerType::WorkflowRunCompleted,
            GithubAutomationTriggerType::Custom,
        ]
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            GithubAutomationTriggerType::PrOpened => "Pull request opened",
            GithubAutomationTriggerType::PrPushed => "Pull request pushed",
            GithubAutomationTriggerType::PrMerged => "Pull request merged",
            GithubAutomationTriggerType::IssueComment => "Issue comment",
            GithubAutomationTriggerType::PrReviewSubmitted => "PR review submitted",
            GithubAutomationTriggerType::WorkflowRunCompleted => "Workflow run completed",
            GithubAutomationTriggerType::Custom => "Custom webhook",
        }
    }

    /// Whether this trigger type uses a custom webhook (and thus surfaces a
    /// one-time hook key on creation).
    pub fn is_custom(&self) -> bool {
        matches!(self, GithubAutomationTriggerType::Custom)
    }

    /// The next trigger type in menu order, wrapping around. Used for the
    /// cycle-on-click selector in the automations form.
    pub fn next(&self) -> GithubAutomationTriggerType {
        let all = Self::all();
        let idx = all.iter().position(|t| t == self).unwrap_or(0);
        all[(idx + 1) % all.len()]
    }
}

/// The kind of action a GitHub automation performs when triggered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubAutomationActionType {
    Prompt,
    Skill,
}

impl GithubAutomationActionType {
    pub fn display_name(&self) -> &'static str {
        match self {
            GithubAutomationActionType::Prompt => "Prompt",
            GithubAutomationActionType::Skill => "Skill",
        }
    }

    /// The other action type (toggles between the two variants).
    pub fn next(&self) -> GithubAutomationActionType {
        match self {
            GithubAutomationActionType::Prompt => GithubAutomationActionType::Skill,
            GithubAutomationActionType::Skill => GithubAutomationActionType::Prompt,
        }
    }
}

/// Trigger configuration for an automation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubAutomationTrigger {
    pub event_type: GithubAutomationTriggerType,
    /// `owner/repo` filter; must intersect the workspace repo allowlist when set.
    pub repo_filter: Option<String>,
    pub branch_pattern: Option<String>,
    pub comment_phrase: Option<String>,
}

/// Action configuration for an automation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubAutomationAction {
    pub action_type: GithubAutomationActionType,
    pub prompt: Option<String>,
    pub skill: Option<String>,
    pub harness: Option<String>,
    pub model_id: Option<String>,
}

/// A single automation record as returned by the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubAutomation {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger: GithubAutomationTrigger,
    pub action: GithubAutomationAction,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// A masked workspace provider key ({provider, last4, addedAt}).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubProviderKey {
    pub provider: String,
    pub last4: String,
    pub added_at: Option<DateTime<Utc>>,
}

/// Combined result of `listGithubAutomations`: the automations plus the masked
/// provider keys configured for the workspace.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListGithubAutomationsData {
    pub automations: Vec<GithubAutomation>,
    pub provider_keys: Vec<GithubProviderKey>,
}

/// Input for creating or updating an automation. `id: None` means create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubAutomationInput {
    pub id: Option<String>,
    pub name: String,
    pub enabled: bool,
    pub trigger: GithubAutomationTrigger,
    pub action: GithubAutomationAction,
}

/// Outcome of an upsert: the stored automation plus the one-time `hook_key`
/// present only when a CUSTOM-trigger automation is first created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpsertGithubAutomationOutcome {
    pub automation: GithubAutomation,
    pub hook_key: Option<String>,
}

// ── Conversions from generated cynic fragment types ──────────────────────────

mod convert {
    use warp_graphql::mutations::{
        set_github_provider_key as m_key, upsert_github_automation as m,
    };
    use warp_graphql::queries::list_github_automations as q;

    use super::*;

    impl From<m_key::GithubProviderKey> for GithubProviderKey {
        fn from(k: m_key::GithubProviderKey) -> Self {
            Self {
                provider: k.provider,
                last4: k.last4,
                added_at: k.added_at.map(|t| t.utc()),
            }
        }
    }

    impl From<q::GithubAutomationTriggerType> for GithubAutomationTriggerType {
        fn from(t: q::GithubAutomationTriggerType) -> Self {
            match t {
                q::GithubAutomationTriggerType::PrOpened => Self::PrOpened,
                q::GithubAutomationTriggerType::PrPushed => Self::PrPushed,
                q::GithubAutomationTriggerType::PrMerged => Self::PrMerged,
                q::GithubAutomationTriggerType::IssueComment => Self::IssueComment,
                q::GithubAutomationTriggerType::PrReviewSubmitted => Self::PrReviewSubmitted,
                q::GithubAutomationTriggerType::WorkflowRunCompleted => Self::WorkflowRunCompleted,
                q::GithubAutomationTriggerType::Custom | q::GithubAutomationTriggerType::Other => {
                    // Unknown trigger types fall back to Custom (never auto-fires
                    // on a known event we can't model).
                    Self::Custom
                }
            }
        }
    }

    impl From<q::GithubAutomationActionType> for GithubAutomationActionType {
        fn from(t: q::GithubAutomationActionType) -> Self {
            match t {
                q::GithubAutomationActionType::Prompt | q::GithubAutomationActionType::Other => {
                    Self::Prompt
                }
                q::GithubAutomationActionType::Skill => Self::Skill,
            }
        }
    }

    impl From<q::GithubAutomationTrigger> for GithubAutomationTrigger {
        fn from(t: q::GithubAutomationTrigger) -> Self {
            Self {
                event_type: t.event_type.into(),
                repo_filter: t.repo_filter,
                branch_pattern: t.branch_pattern,
                comment_phrase: t.comment_phrase,
            }
        }
    }

    impl From<q::GithubAutomationAction> for GithubAutomationAction {
        fn from(a: q::GithubAutomationAction) -> Self {
            Self {
                action_type: a.action_type.into(),
                prompt: a.prompt,
                skill: a.skill,
                harness: a.harness,
                model_id: a.model_id,
            }
        }
    }

    impl From<q::GithubAutomation> for GithubAutomation {
        fn from(a: q::GithubAutomation) -> Self {
            Self {
                id: a.id.into_inner(),
                name: a.name,
                enabled: a.enabled,
                trigger: a.trigger.into(),
                action: a.action.into(),
                created_at: a.created_at.map(|t| t.utc()),
                updated_at: a.updated_at.map(|t| t.utc()),
            }
        }
    }

    impl From<q::GithubProviderKey> for GithubProviderKey {
        fn from(k: q::GithubProviderKey) -> Self {
            Self {
                provider: k.provider,
                last4: k.last4,
                added_at: k.added_at.map(|t| t.utc()),
            }
        }
    }

    impl From<m::GithubAutomationTrigger> for GithubAutomationTrigger {
        fn from(t: m::GithubAutomationTrigger) -> Self {
            let event_type = match t.event_type {
                m::GithubAutomationTriggerType::PrOpened => GithubAutomationTriggerType::PrOpened,
                m::GithubAutomationTriggerType::PrPushed => GithubAutomationTriggerType::PrPushed,
                m::GithubAutomationTriggerType::PrMerged => GithubAutomationTriggerType::PrMerged,
                m::GithubAutomationTriggerType::IssueComment => {
                    GithubAutomationTriggerType::IssueComment
                }
                m::GithubAutomationTriggerType::PrReviewSubmitted => {
                    GithubAutomationTriggerType::PrReviewSubmitted
                }
                m::GithubAutomationTriggerType::WorkflowRunCompleted => {
                    GithubAutomationTriggerType::WorkflowRunCompleted
                }
                m::GithubAutomationTriggerType::Custom => GithubAutomationTriggerType::Custom,
            };
            Self {
                event_type,
                repo_filter: t.repo_filter,
                branch_pattern: t.branch_pattern,
                comment_phrase: t.comment_phrase,
            }
        }
    }

    impl From<m::GithubAutomationAction> for GithubAutomationAction {
        fn from(a: m::GithubAutomationAction) -> Self {
            let action_type = match a.action_type {
                m::GithubAutomationActionType::Prompt => GithubAutomationActionType::Prompt,
                m::GithubAutomationActionType::Skill => GithubAutomationActionType::Skill,
            };
            Self {
                action_type,
                prompt: a.prompt,
                skill: a.skill,
                harness: a.harness,
                model_id: a.model_id,
            }
        }
    }

    impl From<m::GithubAutomation> for GithubAutomation {
        fn from(a: m::GithubAutomation) -> Self {
            Self {
                id: a.id.into_inner(),
                name: a.name,
                enabled: a.enabled,
                trigger: a.trigger.into(),
                action: a.action.into(),
                created_at: None,
                updated_at: None,
            }
        }
    }

    // ── Domain input -> cynic input ──

    impl GithubAutomationTriggerType {
        pub(crate) fn to_gql_input(self) -> m::GithubAutomationTriggerType {
            match self {
                Self::PrOpened => m::GithubAutomationTriggerType::PrOpened,
                Self::PrPushed => m::GithubAutomationTriggerType::PrPushed,
                Self::PrMerged => m::GithubAutomationTriggerType::PrMerged,
                Self::IssueComment => m::GithubAutomationTriggerType::IssueComment,
                Self::PrReviewSubmitted => m::GithubAutomationTriggerType::PrReviewSubmitted,
                Self::WorkflowRunCompleted => m::GithubAutomationTriggerType::WorkflowRunCompleted,
                Self::Custom => m::GithubAutomationTriggerType::Custom,
            }
        }
    }

    impl GithubAutomationActionType {
        pub(crate) fn to_gql_input(self) -> m::GithubAutomationActionType {
            match self {
                Self::Prompt => m::GithubAutomationActionType::Prompt,
                Self::Skill => m::GithubAutomationActionType::Skill,
            }
        }
    }
}
