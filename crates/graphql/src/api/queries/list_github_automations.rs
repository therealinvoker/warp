//! `listGithubAutomations` query: returns a workspace's GitHub automations plus
//! the masked provider keys configured for it.

use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct ListGithubAutomationsVariables {
    pub request_context: RequestContext,
    pub input: ListGithubAutomationsInput,
}

#[derive(cynic::InputObject, Debug)]
pub struct ListGithubAutomationsInput {
    pub workspace_uid: cynic::Id,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubAutomationTriggerType {
    PrOpened,
    PrPushed,
    PrMerged,
    IssueComment,
    PrReviewSubmitted,
    WorkflowRunCompleted,
    Custom,
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubAutomationActionType {
    Prompt,
    Skill,
    #[cynic(fallback)]
    Other,
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
    pub created_at: Option<Time>,
    pub updated_at: Option<Time>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct GithubProviderKey {
    pub provider: String,
    #[cynic(rename = "last4")]
    pub last4: String,
    pub added_at: Option<Time>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ListGithubAutomationsOutput {
    pub automations: Vec<GithubAutomation>,
    pub provider_keys: Vec<GithubProviderKey>,
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ListGithubAutomationsResult {
    ListGithubAutomationsOutput(ListGithubAutomationsOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "ListGithubAutomationsVariables"
)]
pub struct ListGithubAutomations {
    #[arguments(input: $input, requestContext: $request_context)]
    pub list_github_automations: ListGithubAutomationsResult,
}

crate::client::define_operation! {
    list_github_automations(ListGithubAutomationsVariables) -> ListGithubAutomations;
}
