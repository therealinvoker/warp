use anyhow::{anyhow, Result};
use async_trait::async_trait;
use cynic::{MutationBuilder, QueryBuilder};
#[cfg(test)]
use mockall::{automock, predicate::*};
use warp_graphql::mutations::add_invite_link_domain_restriction::{
    AddInviteLinkDomainRestriction, AddInviteLinkDomainRestrictionInput,
    AddInviteLinkDomainRestrictionResult, AddInviteLinkDomainRestrictionVariables,
};
use warp_graphql::mutations::create_team::{
    CreateTeam, CreateTeamInput, CreateTeamResult, CreateTeamVariables,
};
use warp_graphql::mutations::delete_invite_link_domain_restriction::{
    DeleteInviteLinkDomainRestriction, DeleteInviteLinkDomainRestrictionInput,
    DeleteInviteLinkDomainRestrictionResult, DeleteInviteLinkDomainRestrictionVariables,
};
use warp_graphql::mutations::delete_team_invite::{
    DeleteTeamInvite, DeleteTeamInviteInput, DeleteTeamInviteResult, DeleteTeamInviteVariables,
};
use warp_graphql::mutations::join_team_with_team_discovery::{
    JoinTeamWithTeamDiscovery, JoinTeamWithTeamDiscoveryInput, JoinTeamWithTeamDiscoveryResult,
    JoinTeamWithTeamDiscoveryVariables, TeamDiscoveryEntrypoint,
};
use warp_graphql::mutations::remove_user_from_team::{
    RemoveUserFromTeam, RemoveUserFromTeamInput, RemoveUserFromTeamResult,
    RemoveUserFromTeamVariables,
};
use warp_graphql::mutations::rename_team::{
    RenameTeam, RenameTeamInput, RenameTeamResult, RenameTeamVariables,
};
use warp_graphql::mutations::reset_invite_links::{
    ResetInviteLinks, ResetInviteLinksInput, ResetInviteLinksResult, ResetInviteLinksVariables,
};
use warp_graphql::mutations::redeem_team_invite_code::{
    RedeemTeamInviteCode, RedeemTeamInviteCodeInput, RedeemTeamInviteCodeResult,
    RedeemTeamInviteCodeVariables,
};
use warp_graphql::mutations::remove_mcp_allowlist_entry::{
    RemoveMCPAllowlistEntry, RemoveMCPAllowlistEntryInput, RemoveMCPAllowlistEntryResult,
    RemoveMCPAllowlistEntryVariables,
};
use warp_graphql::mutations::send_team_invite_email::{
    SendTeamInviteEmail, SendTeamInviteEmailInput, SendTeamInviteEmailResult,
    SendTeamInviteEmailVariables,
};
use warp_graphql::mutations::set_is_invite_link_enabled::{
    SetIsInviteLinkEnabled, SetIsInviteLinkEnabledInput, SetIsInviteLinkEnabledResult,
    SetIsInviteLinkEnabledVariables,
};
use warp_graphql::mutations::set_team_discoverability::{
    SetTeamDiscoverability, SetTeamDiscoverabilityInput, SetTeamDiscoverabilityResult,
    SetTeamDiscoverabilityVariables,
};
use warp_graphql::mutations::set_team_member_role::{
    SetTeamMemberRole, SetTeamMemberRoleInput, SetTeamMemberRoleResult, SetTeamMemberRoleVariables,
};
use warp_graphql::mutations::transfer_team_ownership::{
    TransferTeamOwnership, TransferTeamOwnershipInput, TransferTeamOwnershipResult,
    TransferTeamOwnershipVariables,
};
use warp_graphql::mutations::update_mcp_governance_settings::{
    UpdateMCPGovernanceSettings, UpdateMCPGovernanceSettingsInput,
    UpdateMCPGovernanceSettingsResult, UpdateMCPGovernanceSettingsVariables,
};
use warp_graphql::mutations::upsert_mcp_allowlist_entry::{
    McpAllowlistEntryInput, UpsertMCPAllowlistEntry, UpsertMCPAllowlistEntryInput,
    UpsertMCPAllowlistEntryResult, UpsertMCPAllowlistEntryVariables,
};
use warp_graphql::queries::get_discoverable_teams::{
    GetDiscoverableTeams, GetDiscoverableTeamsVariables,
};
use warp_graphql::queries::get_workspaces_metadata_for_user::{
    GetWorkspacesMetadataForUser, GetWorkspacesMetadataForUserVariables, PricingInfoResult,
};

use super::ServerApi;
use crate::auth::UserUid;
use crate::cloud_object::CloudObjectEventEntrypoint;
use crate::server::graphql::{get_request_context, get_user_facing_error_message};
use crate::server::ids::ServerId;
use crate::workspaces::team::{DiscoverableTeam, MembershipRole};
use crate::workspaces::user_workspaces::{CreateTeamResponse, WorkspacesMetadataWithPricing};
use crate::workspaces::workspace::{
    McpAllowlistEntryKind, McpGovernanceMode, Workspace, WorkspaceUid,
};

/// Marker substring for op-not-implemented failures; see
/// [`map_unsupported_op_error`]. UIs match on this to render an inline
/// "server does not support this yet" state instead of a generic error.
pub const SERVER_OP_NOT_SUPPORTED: &str = "not supported by the server yet";

/// The backend replies `{"data": {}}` for GraphQL ops it doesn't implement,
/// which surfaces client-side as a decode error for the op's missing root
/// field ("missing field ..."), or as "missing response data" when `data` is
/// null. Map those shapes to a stable, user-facing message tagged with
/// [`SERVER_OP_NOT_SUPPORTED`]; all other errors pass through unchanged.
fn map_unsupported_op_error(err: anyhow::Error, op_display_name: &str) -> anyhow::Error {
    let rendered = format!("{err:#}");
    if rendered.contains("missing field") || rendered.contains("missing response data") {
        anyhow!("{op_display_name} is {SERVER_OP_NOT_SUPPORTED}")
    } else {
        err
    }
}

/// Partial update of a workspace's MCP governance settings. `None` fields are
/// left unchanged by the server.
#[derive(Clone, Debug, Default)]
pub struct McpGovernanceSettingsUpdate {
    pub mode: Option<McpGovernanceMode>,
    pub allow_file_based_servers: Option<bool>,
    pub allow_plugin_import: Option<bool>,
}

/// One allowlist entry to insert or update. The entry id is server-assigned;
/// the server dedupes on (kind, value).
#[derive(Clone, Debug)]
pub struct McpAllowlistEntryUpsert {
    pub kind: McpAllowlistEntryKind,
    pub value: String,
    pub pinned_version: Option<String>,
    pub display_name: Option<String>,
}

#[cfg_attr(test, automock)]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait TeamClient: 'static + Send + Sync {
    async fn workspaces_metadata(&self) -> Result<WorkspacesMetadataWithPricing>;

    async fn add_invite_link_domain_restriction(
        &self,
        team_uid: ServerId,
        domain: String,
    ) -> Result<WorkspacesMetadataWithPricing>;

    async fn delete_invite_link_domain_restriction(
        &self,
        team_uid: ServerId,
        domain_uid: ServerId,
    ) -> Result<WorkspacesMetadataWithPricing>;

    /// Creates a team and returns the result from the server with the newly created team.
    async fn create_team(
        &self,
        name: String,
        entrypoint: CloudObjectEventEntrypoint,
        discoverable: Option<bool>,
    ) -> Result<CreateTeamResponse>;

    /// Joins the team whose invite code (from an invite link or email
    /// invite) is provided, returning the joined workspace like
    /// [`Self::create_team`] does.
    async fn redeem_team_invite_code(
        &self,
        invite_code: String,
        entrypoint: CloudObjectEventEntrypoint,
    ) -> Result<CreateTeamResponse>;

    /// Removes the user from the selected team and returns a list of all teams that a user is
    /// still a member of (including updated team members).
    async fn remove_user_from_team(
        &self,
        user_uid: UserUid,
        team_uid: ServerId,
        entrypoint: CloudObjectEventEntrypoint,
    ) -> Result<WorkspacesMetadataWithPricing>;

    /// Removes the _current_ user from the team (user leaving the team) and returns the list of
    /// all teams that the current user is still a member of.
    async fn leave_team(
        &self,
        user_uid: UserUid,
        team_uid: ServerId,
        entrypoint: CloudObjectEventEntrypoint,
    ) -> Result<WorkspacesMetadataWithPricing>;

    async fn join_team_with_team_discovery(
        &self,
        team_uid: ServerId,
    ) -> Result<WorkspacesMetadataWithPricing>;

    async fn send_team_invite_email(
        &self,
        team_uid: ServerId,
        email: String,
    ) -> Result<WorkspacesMetadataWithPricing>;

    async fn delete_team_invite(
        &self,
        team_uid: ServerId,
        email: String,
    ) -> Result<WorkspacesMetadataWithPricing>;

    async fn get_discoverable_teams(&self) -> Result<Vec<DiscoverableTeam>>;

    async fn rename_team(
        &self,
        new_name: String,
        team_uid: ServerId,
    ) -> Result<WorkspacesMetadataWithPricing>;

    async fn reset_invite_links(&self, team_uid: ServerId)
        -> Result<WorkspacesMetadataWithPricing>;

    async fn set_is_invite_link_enabled(
        &self,
        team_uid: ServerId,
        new_value: bool,
    ) -> Result<WorkspacesMetadataWithPricing>;

    async fn set_team_discoverability(
        &self,
        team_uid: ServerId,
        discoverable: bool,
    ) -> Result<WorkspacesMetadataWithPricing>;

    async fn transfer_team_ownership(
        &self,
        new_owner_email: String,
    ) -> Result<WorkspacesMetadataWithPricing>;

    async fn set_team_member_role(
        &self,
        user_uid: UserUid,
        team_uid: ServerId,
        role: MembershipRole,
    ) -> Result<WorkspacesMetadataWithPricing>;

    /// Admin-only: partially updates a workspace's MCP governance settings
    /// (mode / file-based / plugin-import). Keyed by the WORKSPACE uid, not
    /// the mirrored team uid.
    async fn update_mcp_governance_settings(
        &self,
        workspace_uid: WorkspaceUid,
        update: McpGovernanceSettingsUpdate,
    ) -> Result<WorkspacesMetadataWithPricing>;

    /// Admin-only: inserts or updates one MCP allowlist entry.
    async fn upsert_mcp_allowlist_entry(
        &self,
        workspace_uid: WorkspaceUid,
        entry: McpAllowlistEntryUpsert,
    ) -> Result<WorkspacesMetadataWithPricing>;

    /// Admin-only: removes one MCP allowlist entry by its server id.
    async fn remove_mcp_allowlist_entry(
        &self,
        workspace_uid: WorkspaceUid,
        entry_id: String,
    ) -> Result<WorkspacesMetadataWithPricing>;
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl TeamClient for ServerApi {
    #[tracing::instrument(skip_all, err, fields(tags.cloud_agent = true))]
    async fn workspaces_metadata(&self) -> Result<WorkspacesMetadataWithPricing> {
        let variables = GetWorkspacesMetadataForUserVariables {
            request_context: get_request_context(),
        };
        let operation = GetWorkspacesMetadataForUser::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        let metadata = match response.user {
            warp_graphql::queries::get_workspaces_metadata_for_user::UserResult::UserOutput(
                user_output,
            ) => user_output.user.into(),
            warp_graphql::queries::get_workspaces_metadata_for_user::UserResult::Unknown => {
                return Err(anyhow!("Unable to fetch workspaces metadata"));
            }
        };

        let pricing_info = match response.pricing_info {
            PricingInfoResult::PricingInfoOutput(pricing_output) => {
                Some(pricing_output.pricing_info)
            }
            PricingInfoResult::Unknown => None,
        };

        Ok(WorkspacesMetadataWithPricing {
            metadata,
            pricing_info,
        })
    }

    async fn add_invite_link_domain_restriction(
        &self,
        team_uid: ServerId,
        domain: String,
    ) -> Result<WorkspacesMetadataWithPricing> {
        let variables = AddInviteLinkDomainRestrictionVariables {
            input: AddInviteLinkDomainRestrictionInput {
                team_uid: team_uid.into(),
                domain,
            },
            request_context: get_request_context(),
        };

        let operation = AddInviteLinkDomainRestriction::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .add_invite_link_domain_restriction;

        match result {
            AddInviteLinkDomainRestrictionResult::AddInviteLinkDomainRestrictionOutput(result) => {
                if !result.success {
                    return Err(anyhow!("failed to add invite link domain restriction"));
                }
            }
            AddInviteLinkDomainRestrictionResult::UserFacingError(user_facing_error) => {
                return Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            AddInviteLinkDomainRestrictionResult::Unknown => {
                return Err(anyhow!(
                    "unknown error while adding invite link domain restriction"
                ))
            }
        }

        self.workspaces_metadata().await
    }

    async fn delete_invite_link_domain_restriction(
        &self,
        team_uid: ServerId,
        domain_uid: ServerId,
    ) -> Result<WorkspacesMetadataWithPricing> {
        let variables = DeleteInviteLinkDomainRestrictionVariables {
            input: DeleteInviteLinkDomainRestrictionInput {
                uid: domain_uid.into(),
                team_uid: team_uid.into(),
            },
            request_context: get_request_context(),
        };
        let operation = DeleteInviteLinkDomainRestriction::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .delete_invite_link_domain_restriction;

        match result {
            DeleteInviteLinkDomainRestrictionResult::DeleteInviteLinkDomainRestrictionOutput(
                result,
            ) => {
                if !result.success {
                    return Err(anyhow!("failed to delete invite link domain restriction"));
                }
            }
            DeleteInviteLinkDomainRestrictionResult::UserFacingError(user_facing_error) => {
                return Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            DeleteInviteLinkDomainRestrictionResult::Unknown => {
                return Err(anyhow!(
                    "unknown error while deleting invite link domain restriction"
                ))
            }
        }

        self.workspaces_metadata().await
    }

    async fn create_team(
        &self,
        name: String,
        entrypoint: CloudObjectEventEntrypoint,
        discoverable: Option<bool>,
    ) -> Result<CreateTeamResponse> {
        let variables = CreateTeamVariables {
            input: CreateTeamInput {
                name,
                entrypoint: entrypoint.into(),
                discoverable: discoverable.unwrap_or(false),
            },
            request_context: get_request_context(),
        };

        let operation = CreateTeam::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .create_team;

        match result {
            CreateTeamResult::CreateTeamOutput(output) => {
                let workspace: Workspace = output.workspace.clone().into();
                if let Some(team) = workspace.teams.first() {
                    Ok(CreateTeamResponse {
                        workspace: workspace.clone(),
                        team: team.clone(),
                    })
                } else {
                    Err(anyhow!("failed to create team"))
                }
            }
            CreateTeamResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            CreateTeamResult::Unknown => Err(anyhow!("unknown error while creating team")),
        }
    }

    async fn redeem_team_invite_code(
        &self,
        invite_code: String,
        entrypoint: CloudObjectEventEntrypoint,
    ) -> Result<CreateTeamResponse> {
        let variables = RedeemTeamInviteCodeVariables {
            input: RedeemTeamInviteCodeInput {
                invite_code,
                entrypoint: entrypoint.into(),
            },
            request_context: get_request_context(),
        };

        let operation = RedeemTeamInviteCode::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await
            .map_err(|err| map_unsupported_op_error(err, "Joining a team by invite code"))?
            .redeem_team_invite_code;

        match result {
            RedeemTeamInviteCodeResult::RedeemTeamInviteCodeOutput(output) => {
                let workspace: Workspace = output.workspace.into();
                if let Some(team) = workspace.teams.first() {
                    Ok(CreateTeamResponse {
                        team: team.clone(),
                        workspace: workspace.clone(),
                    })
                } else {
                    Err(anyhow!("joined workspace is missing its team"))
                }
            }
            RedeemTeamInviteCodeResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            RedeemTeamInviteCodeResult::Unknown => {
                Err(anyhow!("unknown error while joining team by invite code"))
            }
        }
    }

    async fn remove_user_from_team(
        &self,
        user_uid: UserUid,
        team_uid: ServerId,
        entrypoint: CloudObjectEventEntrypoint,
    ) -> Result<WorkspacesMetadataWithPricing> {
        let variables = RemoveUserFromTeamVariables {
            input: RemoveUserFromTeamInput {
                user_uid: user_uid.as_str().into(),
                team_uid: team_uid.into(),
                entrypoint: entrypoint.into(),
            },
            request_context: get_request_context(),
        };

        let operation = RemoveUserFromTeam::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .remove_user_from_team;

        match result {
            RemoveUserFromTeamResult::RemoveUserFromTeamOutput(output) => {
                if !output.success {
                    return Err(anyhow!("failed to remove user from team"));
                } else {
                    self.workspaces_metadata().await
                }
            }
            RemoveUserFromTeamResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            RemoveUserFromTeamResult::Unknown => {
                Err(anyhow!("unknown error while removing user from team"))
            }
        }
    }

    async fn leave_team(
        &self,
        user_uid: UserUid,
        team_uid: ServerId,
        entrypoint: CloudObjectEventEntrypoint,
    ) -> Result<WorkspacesMetadataWithPricing> {
        let variables = RemoveUserFromTeamVariables {
            input: RemoveUserFromTeamInput {
                user_uid: user_uid.into(),
                team_uid: team_uid.into(),
                entrypoint: entrypoint.into(),
            },
            request_context: get_request_context(),
        };

        let operation = RemoveUserFromTeam::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .remove_user_from_team;

        match result {
            RemoveUserFromTeamResult::RemoveUserFromTeamOutput(output) => {
                if !output.success {
                    return Err(anyhow!("failed to leave team"));
                } else {
                    self.workspaces_metadata().await
                }
            }
            RemoveUserFromTeamResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            RemoveUserFromTeamResult::Unknown => Err(anyhow!("unknown error while leaving team")),
        }
    }

    async fn join_team_with_team_discovery(
        &self,
        team_uid: ServerId,
    ) -> Result<WorkspacesMetadataWithPricing> {
        let variables = JoinTeamWithTeamDiscoveryVariables {
            input: JoinTeamWithTeamDiscoveryInput {
                team_uid: team_uid.into(),
                entrypoint: TeamDiscoveryEntrypoint::TeamSettings,
            },
            request_context: get_request_context(),
        };

        let operation = JoinTeamWithTeamDiscovery::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .join_team_with_team_discovery;

        match result {
            JoinTeamWithTeamDiscoveryResult::JoinTeamWithTeamDiscoveryOutput(output) => {
                if !output.success {
                    return Err(anyhow!("failed to join team"));
                } else {
                    self.workspaces_metadata().await
                }
            }
            JoinTeamWithTeamDiscoveryResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            JoinTeamWithTeamDiscoveryResult::Unknown => {
                Err(anyhow!("unknown error while joining team"))
            }
        }
    }

    async fn send_team_invite_email(
        &self,
        team_uid: ServerId,
        email: String,
    ) -> Result<WorkspacesMetadataWithPricing> {
        let variables = SendTeamInviteEmailVariables {
            input: SendTeamInviteEmailInput {
                team_uid: team_uid.into(),
                email,
            },
            request_context: get_request_context(),
        };

        let operation = SendTeamInviteEmail::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .send_team_invite_email;

        match result {
            SendTeamInviteEmailResult::SendTeamInviteEmailOutput(output) => {
                if !output.success {
                    return Err(anyhow!("failed to send team invite"));
                } else {
                    self.workspaces_metadata().await
                }
            }
            SendTeamInviteEmailResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            SendTeamInviteEmailResult::Unknown => {
                Err(anyhow!("unknown error while sending team invite"))
            }
        }
    }

    async fn delete_team_invite(
        &self,
        team_uid: ServerId,
        email: String,
    ) -> Result<WorkspacesMetadataWithPricing> {
        let variables = DeleteTeamInviteVariables {
            input: DeleteTeamInviteInput {
                team_uid: team_uid.into(),
                email,
            },
            request_context: get_request_context(),
        };

        let operation = DeleteTeamInvite::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .delete_team_invite;

        match result {
            DeleteTeamInviteResult::DeleteTeamInviteOutput(output) => {
                if !output.success {
                    return Err(anyhow!("failed to delete team invite"));
                } else {
                    self.workspaces_metadata().await
                }
            }
            DeleteTeamInviteResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            DeleteTeamInviteResult::Unknown => {
                Err(anyhow!("unknown error while deleting team invite"))
            }
        }
    }

    async fn get_discoverable_teams(&self) -> Result<Vec<DiscoverableTeam>, anyhow::Error> {
        let variables = GetDiscoverableTeamsVariables {
            request_context: get_request_context(),
        };
        let operation = GetDiscoverableTeams::build(variables);
        let result = self.send_graphql_request(operation, None).await?;

        match result.user {
            warp_graphql::queries::get_discoverable_teams::UserResult::UserOutput(user_output) => {
                Ok(user_output
                    .user
                    .discoverable_teams
                    .into_iter()
                    .map(|gql_team_data| Ok(gql_team_data.into()))
                    .collect::<Result<Vec<DiscoverableTeam>>>()?)
            }
            warp_graphql::queries::get_discoverable_teams::UserResult::UserFacingError(
                user_facing_error,
            ) => Err(anyhow!(get_user_facing_error_message(user_facing_error))),
            warp_graphql::queries::get_discoverable_teams::UserResult::Unknown => {
                Err(anyhow!("unknown error while getting discoverable teams"))
            }
        }
    }

    async fn rename_team(
        &self,
        new_name: String,
        team_uid: ServerId,
    ) -> Result<WorkspacesMetadataWithPricing> {
        let variables = RenameTeamVariables {
            input: RenameTeamInput {
                new_name,
                team_uid: team_uid.into(),
            },
            request_context: get_request_context(),
        };
        let operation = RenameTeam::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .rename_team;

        match result {
            RenameTeamResult::RenameTeamOutput(output) => {
                if output.success {
                    self.workspaces_metadata().await
                } else {
                    Err(anyhow!("failed to rename team"))
                }
            }
            RenameTeamResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            RenameTeamResult::Unknown => Err(anyhow!("unknown error while renaming team")),
        }
    }

    async fn reset_invite_links(
        &self,
        team_uid: ServerId,
    ) -> Result<WorkspacesMetadataWithPricing> {
        let variables = ResetInviteLinksVariables {
            input: ResetInviteLinksInput {
                team_uid: team_uid.into(),
            },
            request_context: get_request_context(),
        };

        let operation = ResetInviteLinks::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .reset_invite_links;

        match result {
            ResetInviteLinksResult::ResetInviteLinksOutput(output) => {
                if output.success {
                    self.workspaces_metadata().await
                } else {
                    Err(anyhow!("failed to reset invite links"))
                }
            }
            ResetInviteLinksResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            ResetInviteLinksResult::Unknown => {
                Err(anyhow!("unknown error while resetting invite links"))
            }
        }
    }

    async fn set_is_invite_link_enabled(
        &self,
        team_uid: ServerId,
        new_value: bool,
    ) -> Result<WorkspacesMetadataWithPricing> {
        let variables = SetIsInviteLinkEnabledVariables {
            input: SetIsInviteLinkEnabledInput {
                team_uid: team_uid.into(),
                new_value,
            },
            request_context: get_request_context(),
        };

        let operation = SetIsInviteLinkEnabled::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .set_is_invite_link_enabled;

        match result {
            SetIsInviteLinkEnabledResult::SetIsInviteLinkEnabledOutput(output) => {
                if output.success {
                    self.workspaces_metadata().await
                } else {
                    Err(anyhow!("failed to set invite link enabled"))
                }
            }
            SetIsInviteLinkEnabledResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            SetIsInviteLinkEnabledResult::Unknown => {
                Err(anyhow!("unknown error while setting invite link enabled"))
            }
        }
    }

    async fn set_team_discoverability(
        &self,
        team_uid: ServerId,
        new_value: bool,
    ) -> Result<WorkspacesMetadataWithPricing> {
        let variables = SetTeamDiscoverabilityVariables {
            input: SetTeamDiscoverabilityInput {
                team_uid: team_uid.into(),
                discoverable: new_value,
            },
            request_context: get_request_context(),
        };

        let operation = SetTeamDiscoverability::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .set_team_discoverability;

        match result {
            SetTeamDiscoverabilityResult::SetTeamDiscoverabilityOutput(output) => {
                if output.success {
                    self.workspaces_metadata().await
                } else {
                    Err(anyhow!("failed to set team discoverability"))
                }
            }
            SetTeamDiscoverabilityResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            SetTeamDiscoverabilityResult::Unknown => {
                Err(anyhow!("unknown error while setting team discoverability"))
            }
        }
    }

    async fn transfer_team_ownership(
        &self,
        new_owner_email: String,
    ) -> Result<WorkspacesMetadataWithPricing> {
        let variables = TransferTeamOwnershipVariables {
            input: TransferTeamOwnershipInput { new_owner_email },
            request_context: get_request_context(),
        };
        let operation = TransferTeamOwnership::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .transfer_team_ownership;

        match result {
            TransferTeamOwnershipResult::TransferTeamOwnershipOutput(output) => {
                if !output.success {
                    return Err(anyhow!("failed to transfer team ownership"));
                } else {
                    self.workspaces_metadata().await
                }
            }
            TransferTeamOwnershipResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            TransferTeamOwnershipResult::Unknown => {
                Err(anyhow!("unknown error while transferring team ownership"))
            }
        }
    }

    async fn set_team_member_role(
        &self,
        user_uid: UserUid,
        team_uid: ServerId,
        role: MembershipRole,
    ) -> Result<WorkspacesMetadataWithPricing> {
        let variables = SetTeamMemberRoleVariables {
            input: SetTeamMemberRoleInput {
                user_uid: user_uid.as_str().into(),
                team_uid: team_uid.into(),
                role: role.into(),
            },
            request_context: get_request_context(),
        };
        let operation = SetTeamMemberRole::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await?
            .set_team_member_role;

        match result {
            SetTeamMemberRoleResult::SetTeamMemberRoleOutput(output) => {
                if output.success {
                    self.workspaces_metadata().await
                } else {
                    Err(anyhow!("failed to set team member role"))
                }
            }
            SetTeamMemberRoleResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            SetTeamMemberRoleResult::Unknown => {
                Err(anyhow!("unknown error while setting team member role"))
            }
        }
    }

    async fn update_mcp_governance_settings(
        &self,
        workspace_uid: WorkspaceUid,
        update: McpGovernanceSettingsUpdate,
    ) -> Result<WorkspacesMetadataWithPricing> {
        let variables = UpdateMCPGovernanceSettingsVariables {
            input: UpdateMCPGovernanceSettingsInput {
                workspace_uid: String::from(workspace_uid).into(),
                mode: update.mode.map(Into::into),
                allow_file_based_servers: update.allow_file_based_servers,
                allow_plugin_import: update.allow_plugin_import,
            },
            request_context: get_request_context(),
        };

        let operation = UpdateMCPGovernanceSettings::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await
            .map_err(|err| map_unsupported_op_error(err, "MCP governance management"))?
            .update_mcp_governance_settings;

        match result {
            UpdateMCPGovernanceSettingsResult::UpdateMCPGovernanceSettingsOutput(_) => {
                // Refetch so the new settings flow through the regular
                // workspace-metadata pipeline (policy recompute, snapshot).
                self.workspaces_metadata().await
            }
            UpdateMCPGovernanceSettingsResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            UpdateMCPGovernanceSettingsResult::Unknown => Err(anyhow!(
                "unknown error while updating MCP governance settings"
            )),
        }
    }

    async fn upsert_mcp_allowlist_entry(
        &self,
        workspace_uid: WorkspaceUid,
        entry: McpAllowlistEntryUpsert,
    ) -> Result<WorkspacesMetadataWithPricing> {
        let variables = UpsertMCPAllowlistEntryVariables {
            input: UpsertMCPAllowlistEntryInput {
                workspace_uid: String::from(workspace_uid).into(),
                entry: McpAllowlistEntryInput {
                    kind: entry.kind.into(),
                    value: entry.value,
                    pinned_version: entry.pinned_version,
                    display_name: entry.display_name,
                },
            },
            request_context: get_request_context(),
        };

        let operation = UpsertMCPAllowlistEntry::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await
            .map_err(|err| map_unsupported_op_error(err, "Allowlist entry management"))?
            .upsert_mcp_allowlist_entry;

        match result {
            UpsertMCPAllowlistEntryResult::UpsertMCPAllowlistEntryOutput(_) => {
                self.workspaces_metadata().await
            }
            UpsertMCPAllowlistEntryResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            UpsertMCPAllowlistEntryResult::Unknown => {
                Err(anyhow!("unknown error while adding MCP allowlist entry"))
            }
        }
    }

    async fn remove_mcp_allowlist_entry(
        &self,
        workspace_uid: WorkspaceUid,
        entry_id: String,
    ) -> Result<WorkspacesMetadataWithPricing> {
        let variables = RemoveMCPAllowlistEntryVariables {
            input: RemoveMCPAllowlistEntryInput {
                workspace_uid: String::from(workspace_uid).into(),
                entry_id: entry_id.into(),
            },
            request_context: get_request_context(),
        };

        let operation = RemoveMCPAllowlistEntry::build(variables);
        let result = self
            .send_graphql_request(operation, None)
            .await
            .map_err(|err| map_unsupported_op_error(err, "Allowlist entry management"))?
            .remove_mcp_allowlist_entry;

        match result {
            RemoveMCPAllowlistEntryResult::RemoveMCPAllowlistEntryOutput(_) => {
                self.workspaces_metadata().await
            }
            RemoveMCPAllowlistEntryResult::UserFacingError(user_facing_error) => {
                Err(anyhow!(get_user_facing_error_message(user_facing_error)))
            }
            RemoveMCPAllowlistEntryResult::Unknown => {
                Err(anyhow!("unknown error while removing MCP allowlist entry"))
            }
        }
    }
}
