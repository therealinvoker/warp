use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;
use crate::workspace::{McpGovernanceMode, McpGovernanceSettings};

// Admin/owner-only update of a workspace's MCP governance override (mode /
// allowFileBasedServers / allowPluginImport). Allowlist entry CRUD goes
// through the dedicated Upsert/RemoveMCPAllowlistEntry mutations.
//
// NOTE: the Rust type names deliberately keep the `MCP` capitalization so the
// operation name sent to the server is exactly `UpdateMCPGovernanceSettings`,
// which is what the backend keys its handler on.

/*
mutation UpdateMCPGovernanceSettings($input: UpdateMCPGovernanceSettingsInput!, $request_context: RequestContext!) {
  updateMCPGovernanceSettings(input: $input, requestContext: $request_context) {
    ... on UpdateMCPGovernanceSettingsOutput {
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
pub struct UpdateMCPGovernanceSettingsVariables {
    pub input: UpdateMCPGovernanceSettingsInput,
    pub request_context: RequestContext,
}

/// Omitted (`None`) fields are left unchanged by the server.
#[derive(cynic::InputObject, Debug)]
pub struct UpdateMCPGovernanceSettingsInput {
    pub workspace_uid: cynic::Id,
    pub mode: Option<McpGovernanceMode>,
    pub allow_file_based_servers: Option<bool>,
    pub allow_plugin_import: Option<bool>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "UpdateMCPGovernanceSettingsVariables"
)]
pub struct UpdateMCPGovernanceSettings {
    #[arguments(input: $input, requestContext: $request_context)]
    #[cynic(rename = "updateMCPGovernanceSettings")]
    pub update_mcp_governance_settings: UpdateMCPGovernanceSettingsResult,
}
crate::client::define_operation! {
    update_mcp_governance_settings(UpdateMCPGovernanceSettingsVariables) -> UpdateMCPGovernanceSettings;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UpdateMCPGovernanceSettingsResult {
    UpdateMCPGovernanceSettingsOutput(UpdateMCPGovernanceSettingsOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpdateMCPGovernanceSettingsOutput {
    pub settings: McpGovernanceSettings,
    pub response_context: ResponseContext,
}
