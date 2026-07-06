//! `setGithubProviderKey` mutation: store an encrypted workspace provider key.
//!
//! The plaintext `key` is sent once and never returned; the server serializes
//! it back only as `{provider, last4, addedAt}`.

use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::InputObject, Debug)]
pub struct SetGithubProviderKeyInput {
    pub workspace_uid: cynic::Id,
    pub provider: String,
    pub key: String,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct SetGithubProviderKeyVariables {
    pub input: SetGithubProviderKeyInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct GithubProviderKey {
    pub provider: String,
    #[cynic(rename = "last4")]
    pub last4: String,
    pub added_at: Option<Time>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct SetGithubProviderKeyOutput {
    pub provider_key: GithubProviderKey,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum SetGithubProviderKeyResult {
    SetGithubProviderKeyOutput(SetGithubProviderKeyOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "SetGithubProviderKeyVariables"
)]
pub struct SetGithubProviderKey {
    #[arguments(input: $input, requestContext: $request_context)]
    pub set_github_provider_key: SetGithubProviderKeyResult,
}

crate::client::define_operation! {
    SetGithubProviderKey(SetGithubProviderKeyVariables) -> SetGithubProviderKey;
}
