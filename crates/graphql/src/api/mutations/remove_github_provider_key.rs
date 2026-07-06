//! `removeGithubProviderKey` mutation: delete a workspace provider key.

use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::InputObject, Debug)]
pub struct RemoveGithubProviderKeyInput {
    pub workspace_uid: cynic::Id,
    pub provider: String,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct RemoveGithubProviderKeyVariables {
    pub input: RemoveGithubProviderKeyInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct RemoveGithubProviderKeyOutput {
    pub success: bool,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum RemoveGithubProviderKeyResult {
    RemoveGithubProviderKeyOutput(RemoveGithubProviderKeyOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "RemoveGithubProviderKeyVariables"
)]
pub struct RemoveGithubProviderKey {
    #[arguments(input: $input, requestContext: $request_context)]
    pub remove_github_provider_key: RemoveGithubProviderKeyResult,
}

crate::client::define_operation! {
    RemoveGithubProviderKey(RemoveGithubProviderKeyVariables) -> RemoveGithubProviderKey;
}
