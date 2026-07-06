//! `removeGithubAutomation` mutation: delete a GitHub automation by id.

use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::InputObject, Debug)]
pub struct RemoveGithubAutomationInput {
    pub workspace_uid: cynic::Id,
    pub id: cynic::Id,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct RemoveGithubAutomationVariables {
    pub input: RemoveGithubAutomationInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct RemoveGithubAutomationOutput {
    pub success: bool,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum RemoveGithubAutomationResult {
    RemoveGithubAutomationOutput(RemoveGithubAutomationOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "RemoveGithubAutomationVariables"
)]
pub struct RemoveGithubAutomation {
    #[arguments(input: $input, requestContext: $request_context)]
    pub remove_github_automation: RemoveGithubAutomationResult,
}

crate::client::define_operation! {
    RemoveGithubAutomation(RemoveGithubAutomationVariables) -> RemoveGithubAutomation;
}
