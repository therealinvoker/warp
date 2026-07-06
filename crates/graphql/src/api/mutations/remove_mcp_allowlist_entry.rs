use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;
use crate::workspace::McpGovernanceSettings;

// Admin/owner-only removal of one MCP allowlist entry by its server-assigned
// id. The backend handler for this op lands in a follow-up: until then the
// server replies `{"data": {}}` and callers surface a "not supported by the
// server yet" state.
//
// NOTE: the Rust type names deliberately keep the `MCP` capitalization so the
// operation name sent to the server is exactly `RemoveMCPAllowlistEntry`.

/*
mutation RemoveMCPAllowlistEntry($input: RemoveMCPAllowlistEntryInput!, $request_context: RequestContext!) {
  removeMCPAllowlistEntry(input: $input, requestContext: $request_context) {
    ... on RemoveMCPAllowlistEntryOutput {
      settings {
        mode
        allowlist {
          id
          kind
          value
          pinnedVersion
          displayName
        }
        allowFileBasedServers
        allowPluginImport
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
pub struct RemoveMCPAllowlistEntryVariables {
    pub input: RemoveMCPAllowlistEntryInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct RemoveMCPAllowlistEntryInput {
    pub workspace_uid: cynic::Id,
    pub entry_id: cynic::Id,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "RemoveMCPAllowlistEntryVariables"
)]
pub struct RemoveMCPAllowlistEntry {
    #[arguments(input: $input, requestContext: $request_context)]
    #[cynic(rename = "removeMCPAllowlistEntry")]
    pub remove_mcp_allowlist_entry: RemoveMCPAllowlistEntryResult,
}
crate::client::define_operation! {
    remove_mcp_allowlist_entry(RemoveMCPAllowlistEntryVariables) -> RemoveMCPAllowlistEntry;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum RemoveMCPAllowlistEntryResult {
    RemoveMCPAllowlistEntryOutput(RemoveMCPAllowlistEntryOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct RemoveMCPAllowlistEntryOutput {
    pub settings: McpGovernanceSettings,
    pub response_context: ResponseContext,
}
