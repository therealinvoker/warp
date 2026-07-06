//! `upsertGithubAutomation` mutation: create or update a GitHub automation.
//!
//! On CUSTOM-trigger creation the server returns the plaintext `hookKey`
//! exactly once; callers must surface it immediately since it is not
//! retrievable afterwards.

use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubAutomationTriggerType {
    PrOpened,
    PrPushed,
    PrMerged,
    IssueComment,
    PrReviewSubmitted,
    WorkflowRunCompleted,
    Custom,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubAutomationActionType {
    Prompt,
    Skill,
}

#[derive(cynic::InputObject, Debug)]
pub struct GithubAutomationTriggerInput {
    pub event_type: GithubAutomationTriggerType,
    pub repo_filter: Option<String>,
    pub branch_pattern: Option<String>,
    pub comment_phrase: Option<String>,
}

#[derive(cynic::InputObject, Debug)]
pub struct GithubAutomationActionInput {
    pub action_type: GithubAutomationActionType,
    pub prompt: Option<String>,
    pub skill: Option<String>,
    pub harness: Option<String>,
    pub model_id: Option<String>,
}

#[derive(cynic::InputObject, Debug)]
pub struct UpsertGithubAutomationInput {
    /// Present when updating an existing automation; omit to create.
    pub id: Option<cynic::Id>,
    pub workspace_uid: cynic::Id,
    pub name: String,
    pub enabled: bool,
    pub trigger: GithubAutomationTriggerInput,
    pub action: GithubAutomationActionInput,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct UpsertGithubAutomationVariables {
    pub input: UpsertGithubAutomationInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct GithubAutomationTrigger {
    pub event_type: GithubAutomationTriggerType,
    pub repo_filter: Option<String>,
    pub branch_pattern: Option<String>,
    pub comment_phrase: Option<String>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct GithubAutomationAction {
    pub action_type: GithubAutomationActionType,
    pub prompt: Option<String>,
    pub skill: Option<String>,
    pub harness: Option<String>,
    pub model_id: Option<String>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct GithubAutomation {
    pub id: cynic::Id,
    pub name: String,
    pub enabled: bool,
    pub trigger: GithubAutomationTrigger,
    pub action: GithubAutomationAction,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpsertGithubAutomationOutput {
    pub automation: GithubAutomation,
    /// Plaintext custom-webhook signing key, present only once on CUSTOM create.
    pub hook_key: Option<String>,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UpsertGithubAutomationResult {
    UpsertGithubAutomationOutput(UpsertGithubAutomationOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "UpsertGithubAutomationVariables"
)]
pub struct UpsertGithubAutomation {
    #[arguments(input: $input, requestContext: $request_context)]
    pub upsert_github_automation: UpsertGithubAutomationResult,
}

crate::client::define_operation! {
    UpsertGithubAutomation(UpsertGithubAutomationVariables) -> UpsertGithubAutomation;
}
