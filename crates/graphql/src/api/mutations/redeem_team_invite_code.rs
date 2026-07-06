use crate::error::UserFacingError;
use crate::object::CloudObjectEventEntrypoint;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;
use crate::workspace::Workspace;

// Joins the team whose invite code (from an invite link or email invite) is
// provided. Mirrors CreateTeam: on success the full Workspace fragment is
// returned so the client can adopt the joined team without a refetch.
// Failures (wrong/expired code, domain restrictions) surface as the
// UserFacingError union variant. The backend handler lands in a follow-up:
// until then the server replies `{"data": {}}` and callers surface a "not
// supported by the server yet" state.

/*
mutation RedeemTeamInviteCode($input: RedeemTeamInviteCodeInput!, $request_context: RequestContext!) {
  redeemTeamInviteCode(input: $input, requestContext: $request_context) {
    ... on RedeemTeamInviteCodeOutput {
      workspace {
        # Full Workspace fragment, identical to CreateTeam's selection.
        ...
      }
      responseContext {
        serverVersion
      }
    }
    ... on UserFacingError {
      error {
        message
      }
      responseContext {
        serverVersion
      }
    }
  }
}
*/

#[derive(cynic::QueryVariables, Debug)]
pub struct RedeemTeamInviteCodeVariables {
    pub input: RedeemTeamInviteCodeInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct RedeemTeamInviteCodeInput {
    pub invite_code: String,
    pub entrypoint: CloudObjectEventEntrypoint,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "RedeemTeamInviteCodeVariables"
)]
pub struct RedeemTeamInviteCode {
    #[arguments(input: $input, requestContext: $request_context)]
    pub redeem_team_invite_code: RedeemTeamInviteCodeResult,
}
crate::client::define_operation! {
    redeem_team_invite_code(RedeemTeamInviteCodeVariables) -> RedeemTeamInviteCode;
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum RedeemTeamInviteCodeResult {
    RedeemTeamInviteCodeOutput(RedeemTeamInviteCodeOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct RedeemTeamInviteCodeOutput {
    pub workspace: Workspace,
    pub response_context: ResponseContext,
}
