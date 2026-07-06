use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;
use crate::workspace::{McpAllowlistEntryKind, McpGovernanceSettings};

// Admin/owner-only insert-or-update of one MCP allowlist entry. The entry id
// is server-assigned; upserts dedupe on (kind, value). The backend handler
// for this op lands in a follow-up: until then the server replies
// `{"data": {}}` and callers surface a "not supported by the server yet"
// state.
//
// NOTE: the Rust type names deliberately keep the `MCP` capitalization so the
// operation name sent to the server is exactly `UpsertMCPAllowlistEntry`.

/*
mutation UpsertMCPAllowlistEntry($input: UpsertMCPAllowlistEntryInput!, $request_context: RequestContext!) {
  upsertMCPAllowlistEntry(input: $input, requestContext: $request_context) {
    ... on UpsertMCPAllowlistEntryOutput {
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
pub struct UpsertMCPAllowlistEntryVariables {
    pub input: UpsertMCPAllowlistEntryInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct UpsertMCPAllowlistEntryInput {
    pub workspace_uid: cynic::Id,
    pub entry: McpAllowlistEntryInput,
}

#[derive(cynic::InputObject, Debug)]
pub struct McpAllowlistEntryInput {
    pub kind: McpAllowlistEntryKind,
    pub value: String,
    pub pinned_version: Option<String>,
    pub display_name: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "UpsertMCPAllowlistEntryVariables"
)]
pub struct UpsertMCPAllowlistEntry {
    #[arguments(input: $input, requestContext: $request_context)]
    #[cynic(rename = "upsertMCPAllowlistEntry")]
    pub upsert_mcp_allowlist_entry: UpsertMCPAllowlistEntryResult,
}
crate::client::define_operation! {
    upsert_mcp_allowlist_entry(UpsertMCPAllowlistEntryVariables) -> UpsertMCPAllowlistEntry;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UpsertMCPAllowlistEntryResult {
    UpsertMCPAllowlistEntryOutput(UpsertMCPAllowlistEntryOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpsertMCPAllowlistEntryOutput {
    pub settings: McpGovernanceSettings,
    pub response_context: ResponseContext,
}
