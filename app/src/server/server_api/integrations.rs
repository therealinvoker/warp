use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use cynic::{MutationBuilder, QueryBuilder};
#[cfg(test)]
use mockall::automock;
use warp_graphql::mutations::create_simple_integration::{
    CreateSimpleIntegration, CreateSimpleIntegrationOutput, CreateSimpleIntegrationResult,
    CreateSimpleIntegrationVariables, SimpleIntegrationConfig,
};
use warp_graphql::queries::get_integrations_using_environment::{
    GetIntegrationsUsingEnvironment, GetIntegrationsUsingEnvironmentInput,
    GetIntegrationsUsingEnvironmentOutput, GetIntegrationsUsingEnvironmentResult,
    GetIntegrationsUsingEnvironmentVariables,
};
use warp_graphql::queries::get_oauth_connect_tx_status::{
    GetOAuthConnectTxStatus, GetOAuthConnectTxStatusInput, GetOAuthConnectTxStatusResult,
    GetOAuthConnectTxStatusVariables, OauthConnectTxStatus,
};
use warp_graphql::queries::get_simple_integrations::{
    SimpleIntegrations, SimpleIntegrationsInput, SimpleIntegrationsOutput,
    SimpleIntegrationsResult, SimpleIntegrationsVariables,
};
use warp_graphql::queries::suggest_cloud_environment_image::{
    RepoInput as SuggestCloudEnvironmentImageRepoInput, SuggestCloudEnvironmentImage,
    SuggestCloudEnvironmentImageInput, SuggestCloudEnvironmentImageResult,
    SuggestCloudEnvironmentImageVariables,
};
use warp_graphql::queries::user_github_info::{
    GithubAuthRequiredOutput, UserGithubInfo, UserGithubInfoResult, UserGithubInfoVariables,
};
use warp_graphql::queries::user_repo_auth_status::{
    RepoInput as UserRepoAuthStatusRepoInput, UserRepoAuthStatus, UserRepoAuthStatusInput,
    UserRepoAuthStatusOutput, UserRepoAuthStatusResult, UserRepoAuthStatusVariables,
};

use super::ServerApi;
use crate::channel::ChannelState;
use crate::features::FeatureFlag;
#[cfg(feature = "github_automations")]
use crate::github::automations::{
    GithubAutomationInput, GithubProviderKey, ListGithubAutomationsData,
    UpsertGithubAutomationOutcome,
};
use crate::server::graphql::{get_request_context, get_user_facing_error_message};

/// Response shape of `GET /api/v1/github/token`.
///
/// The server mints a short-lived user-to-server GitHub token (refreshed
/// server-side, issuance audit-logged) so the client can call api.github.com
/// directly without holding GitHub App credentials. `expires_at` is an RFC3339
/// timestamp; `installation_id` identifies the GitHub App installation.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct GithubTokenResponse {
    pub token: String,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub installation_id: Option<u64>,
}

#[cfg(not(target_family = "wasm"))]
pub trait IntegrationsClientBounds: Send + Sync {}

#[cfg(not(target_family = "wasm"))]
impl<T: 'static + Send + Sync> IntegrationsClientBounds for T {}

#[cfg(target_family = "wasm")]
pub trait IntegrationsClientBounds {}

#[cfg(target_family = "wasm")]
impl<T: 'static> IntegrationsClientBounds for T {}

#[cfg_attr(test, automock)]
#[cfg_attr(target_family = "wasm", allow(dead_code))]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
pub trait IntegrationsClient: 'static + IntegrationsClientBounds {
    /// Checks the user's GitHub authorization status for the given repositories.
    ///
    /// Returns a list of statuses for each repo, indicating whether the user has
    /// access to the repo, and an optional auth URL for the user to authorize.
    async fn check_user_repo_auth_status(
        &self,
        repos: Vec<(String, String)>,
    ) -> Result<UserRepoAuthStatusOutput>;

    /// Creates or updates a simple integration on the server.
    ///
    /// # Arguments
    /// * `integration_type` - The type of integration (e.g. "github", "linear", "slack")
    /// * `is_update` - Whether this is an update to an existing integration
    /// * `environment_uid` - The UID of the environment to associate with this integration
    /// * `base_prompt` - Optional base prompt for the integration
    /// * `model_id` - Optional model ID for the integration
    /// * `mcp_servers_json` - Optional JSON string encoding a map[string]MCPServerConfig (ambient agent spec)
    /// * `remove_mcp_server_names` - Optional list of MCP server names to remove (applies on update)
    /// * `worker_host` - Optional worker host ID for self-hosted workers
    /// * `enabled` - Whether the integration should be enabled on creation
    #[allow(clippy::too_many_arguments)]
    async fn create_or_update_simple_integration(
        &self,
        integration_type: String,
        is_update: bool,
        environment_uid: Option<String>,
        base_prompt: Option<String>,
        model_id: Option<String>,
        mcp_servers_json: Option<String>,
        remove_mcp_server_names: Option<Vec<String>>,
        worker_host: Option<String>,
        enabled: bool,
    ) -> Result<CreateSimpleIntegrationOutput>;

    /// Lists simple integrations for a fixed set of provider slugs.
    ///
    /// The server will return one SimpleIntegration entry per requested provider,
    /// regardless of whether the connection or integration currently exists.
    async fn list_simple_integrations(
        &self,
        providers: Vec<String>,
    ) -> Result<SimpleIntegrationsOutput>;

    /// Polls the status of an OAuth connect transaction.
    ///
    /// # Arguments
    /// * `tx_id` - The transaction ID returned from create_simple_integration
    ///
    /// # Returns
    /// * `Ok(OauthConnectTxStatus)` - The current status of the transaction
    /// * `Err` - If the transaction is not found or polling fails
    async fn poll_oauth_connect_status(&self, tx_id: String) -> Result<OauthConnectTxStatus>;

    /// Gets the list of integration provider names that are using the specified environment.
    ///
    /// # Arguments
    /// * `environment_id` - The ID of the environment to check
    ///
    /// # Returns
    /// * `Ok(Vec<String>)` - List of provider names (e.g., ["linear", "slack"]) using this environment
    /// * `Err` - If the query fails
    async fn get_integrations_using_environment(
        &self,
        environment_id: String,
    ) -> Result<GetIntegrationsUsingEnvironmentOutput>;

    /// Gets the user's GitHub connection info, including accessible repos.
    ///
    /// # Returns
    /// * `Ok(UserGithubInfoResult)` - Either connected with repos, or auth required
    /// * `Err` - If the query fails
    async fn get_user_github_info(&self) -> Result<UserGithubInfoResult>;

    /// Fetches a short-lived GitHub token from the backend
    /// (`GET /api/v1/github/token`) for direct api.github.com access.
    ///
    /// The backend mints/refreshes a user-to-server token server-side and
    /// audit-logs issuance. Callers (`GithubConnection`) cache the token in
    /// memory and treat `expires_at` strictly.
    ///
    /// TODO(G4): governance is enforced server-side at token minting; the
    /// client also gates token requests as defense-in-depth (see
    /// `GithubConnection::token`).
    async fn get_github_token(&self) -> Result<GithubTokenResponse>;

    /// Suggests a Docker image for a cloud environment based on the provided repos.
    async fn suggest_cloud_environment_image(
        &self,
        repos: Vec<(String, String)>,
    ) -> Result<SuggestCloudEnvironmentImageResult>;

    /// Lists the GitHub automations and masked provider keys for a workspace.
    ///
    /// Gated on `FeatureFlag::GithubAutomations` at the call site.
    #[cfg(feature = "github_automations")]
    async fn list_github_automations(
        &self,
        workspace_uid: String,
    ) -> Result<ListGithubAutomationsData>;

    /// Creates or updates a GitHub automation.
    ///
    /// Returns the stored automation and, on CUSTOM-trigger creation, the
    /// plaintext `hook_key` (surfaced exactly once).
    #[cfg(feature = "github_automations")]
    async fn upsert_github_automation(
        &self,
        workspace_uid: String,
        input: GithubAutomationInput,
    ) -> Result<UpsertGithubAutomationOutcome>;

    /// Removes a GitHub automation by id.
    #[cfg(feature = "github_automations")]
    async fn remove_github_automation(&self, workspace_uid: String, id: String) -> Result<()>;

    /// Sets (creates or replaces) a workspace GitHub provider key. The plaintext
    /// `key` is sent once; the server returns only the masked `{provider,last4}`.
    #[cfg(feature = "github_automations")]
    async fn set_github_provider_key(
        &self,
        workspace_uid: String,
        provider: String,
        key: String,
    ) -> Result<GithubProviderKey>;

    /// Removes a workspace GitHub provider key.
    #[cfg(feature = "github_automations")]
    async fn remove_github_provider_key(
        &self,
        workspace_uid: String,
        provider: String,
    ) -> Result<()>;
}

#[cfg_attr(target_family = "wasm", async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
impl IntegrationsClient for ServerApi {
    async fn check_user_repo_auth_status(
        &self,
        repos: Vec<(String, String)>,
    ) -> Result<UserRepoAuthStatusOutput> {
        let repo_inputs: Vec<UserRepoAuthStatusRepoInput> = repos
            .into_iter()
            .map(|(owner, repo)| UserRepoAuthStatusRepoInput { owner, repo })
            .collect();

        let variables = UserRepoAuthStatusVariables {
            request_context: get_request_context(),
            input: UserRepoAuthStatusInput { repos: repo_inputs },
        };

        let operation = UserRepoAuthStatus::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.user_repo_auth_status {
            UserRepoAuthStatusResult::UserRepoAuthStatusOutput(output) => Ok(output),
            UserRepoAuthStatusResult::Unknown => Err(anyhow::anyhow!(
                "Failed to check GitHub auth status: unknown response"
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_or_update_simple_integration(
        &self,
        integration_type: String,
        is_update: bool,
        environment_uid: Option<String>,
        base_prompt: Option<String>,
        model_id: Option<String>,
        mcp_servers_json: Option<String>,
        remove_mcp_server_names: Option<Vec<String>>,
        worker_host: Option<String>,
        enabled: bool,
    ) -> Result<CreateSimpleIntegrationOutput> {
        let variables = CreateSimpleIntegrationVariables {
            config: SimpleIntegrationConfig {
                base_prompt,
                environment_uid,
                model_id,
                mcp_servers_json,
                remove_mcp_server_names,
                worker_host,
            },
            enabled,
            integration_type,
            is_update,
            request_context: get_request_context(),
        };

        let operation = CreateSimpleIntegration::build(variables);
        let response = self.send_graphql_request(operation, None).await?;
        match response.create_simple_integration {
            CreateSimpleIntegrationResult::CreateSimpleIntegrationOutput(output) => Ok(output),
            CreateSimpleIntegrationResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            CreateSimpleIntegrationResult::Unknown => {
                Err(anyhow!("Unknown error while creating integration"))
            }
        }
    }

    async fn get_integrations_using_environment(
        &self,
        environment_id: String,
    ) -> Result<GetIntegrationsUsingEnvironmentOutput> {
        let variables = GetIntegrationsUsingEnvironmentVariables {
            request_context: get_request_context(),
            input: GetIntegrationsUsingEnvironmentInput { environment_id },
        };

        let operation = GetIntegrationsUsingEnvironment::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.get_integrations_using_environment {
            GetIntegrationsUsingEnvironmentResult::GetIntegrationsUsingEnvironmentOutput(
                output,
            ) => Ok(output),
            GetIntegrationsUsingEnvironmentResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            GetIntegrationsUsingEnvironmentResult::Unknown => Err(anyhow!(
                "Unknown error while getting integrations using environment"
            )),
        }
    }

    async fn list_simple_integrations(
        &self,
        providers: Vec<String>,
    ) -> Result<SimpleIntegrationsOutput> {
        let variables = SimpleIntegrationsVariables {
            request_context: get_request_context(),
            input: SimpleIntegrationsInput { providers },
        };

        let operation = SimpleIntegrations::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.simple_integrations {
            SimpleIntegrationsResult::SimpleIntegrationsOutput(output) => Ok(output),
            SimpleIntegrationsResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            SimpleIntegrationsResult::Unknown => {
                Err(anyhow!("Unknown error while listing simple integrations"))
            }
        }
    }

    async fn poll_oauth_connect_status(&self, tx_id: String) -> Result<OauthConnectTxStatus> {
        let variables = GetOAuthConnectTxStatusVariables {
            request_context: get_request_context(),
            input: GetOAuthConnectTxStatusInput {
                tx_id: cynic::Id::new(tx_id),
            },
        };

        let operation = GetOAuthConnectTxStatus::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.get_oauth_connect_tx_status {
            GetOAuthConnectTxStatusResult::GetOAuthConnectTxStatusOutput(output) => {
                Ok(output.status)
            }
            GetOAuthConnectTxStatusResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            GetOAuthConnectTxStatusResult::Unknown => {
                Err(anyhow!("Unknown error while polling OAuth status"))
            }
        }
    }

    async fn get_user_github_info(&self) -> Result<UserGithubInfoResult> {
        let variables = UserGithubInfoVariables {
            request_context: get_request_context(),
        };

        let operation = UserGithubInfo::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        let result = response.user_github_info;

        // Dev-only helper for testing GitHub-unauthed flows.
        //
        // Important: this runs after the network request completes so the UI can still
        // show the loading state.
        if FeatureFlag::SimulateGithubUnauthed.is_enabled() {
            if let UserGithubInfoResult::GithubConnectedOutput(connected) = &result {
                let auth_url = format!("{}/oauth/connect/github", ChannelState::server_root_url());
                return Ok(UserGithubInfoResult::GithubAuthRequiredOutput(
                    GithubAuthRequiredOutput {
                        auth_url,
                        // This value is unused by the app UI; it exists in the schema for
                        // tx-bound flows. We intentionally omit txId from the auth URL so
                        // the web flow can proceed without a server-created tx.
                        tx_id: cynic::Id::new("simulated"),
                        app_install_link: connected.app_install_link.clone(),
                    },
                ));
            }
        }

        Ok(result)
    }

    async fn get_github_token(&self) -> Result<GithubTokenResponse> {
        // TODO(G4): defense-in-depth governance check would gate here before
        // requesting a token (mirroring how McpGovernance gates spawns). The
        // authoritative enforcement is server-side at token minting.
        let auth_token = self
            .get_or_refresh_access_token()
            .await
            .context("Failed to get access token for GitHub token request")?;

        let url = format!("{}/api/v1/github/token", ChannelState::server_root_url());

        let mut request = self.base_client.http_client().get(&url);
        if let Some(token) = auth_token.as_bearer_token() {
            request = request.bearer_auth(token);
        }
        for (name, value) in self.ambient_agent_headers().await? {
            request = request.header(name, value);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("Failed to send GitHub token request to {url}"))?;

        if !response.status().is_success() {
            return Err(Self::error_from_response(response).await);
        }

        response
            .json::<GithubTokenResponse>()
            .await
            .context("Failed to deserialize GitHub token response")
    }

    async fn suggest_cloud_environment_image(
        &self,
        repos: Vec<(String, String)>,
    ) -> Result<SuggestCloudEnvironmentImageResult> {
        let repo_inputs: Vec<SuggestCloudEnvironmentImageRepoInput> = repos
            .into_iter()
            .map(|(owner, repo)| SuggestCloudEnvironmentImageRepoInput { owner, repo })
            .collect();

        let variables = SuggestCloudEnvironmentImageVariables {
            request_context: get_request_context(),
            input: SuggestCloudEnvironmentImageInput { repos: repo_inputs },
        };

        let operation = SuggestCloudEnvironmentImage::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.suggest_cloud_environment_image {
            SuggestCloudEnvironmentImageResult::SuggestCloudEnvironmentImageAuthRequiredOutput(
                output,
            ) => Ok(
                SuggestCloudEnvironmentImageResult::SuggestCloudEnvironmentImageAuthRequiredOutput(
                    output,
                ),
            ),
            SuggestCloudEnvironmentImageResult::SuggestCloudEnvironmentImageOutput(output) => {
                Ok(SuggestCloudEnvironmentImageResult::SuggestCloudEnvironmentImageOutput(output))
            }
            SuggestCloudEnvironmentImageResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            SuggestCloudEnvironmentImageResult::Unknown => Err(anyhow!(
                "Unknown response from suggestCloudEnvironmentImage query"
            )),
        }
    }

    #[cfg(feature = "github_automations")]
    async fn list_github_automations(
        &self,
        workspace_uid: String,
    ) -> Result<ListGithubAutomationsData> {
        use warp_graphql::queries::list_github_automations::{
            ListGithubAutomations, ListGithubAutomationsInput, ListGithubAutomationsResult,
            ListGithubAutomationsVariables,
        };

        let variables = ListGithubAutomationsVariables {
            request_context: get_request_context(),
            input: ListGithubAutomationsInput {
                workspace_uid: cynic::Id::new(workspace_uid),
            },
        };
        let operation = ListGithubAutomations::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.list_github_automations {
            ListGithubAutomationsResult::ListGithubAutomationsOutput(output) => {
                Ok(ListGithubAutomationsData {
                    automations: output.automations.into_iter().map(Into::into).collect(),
                    provider_keys: output.provider_keys.into_iter().map(Into::into).collect(),
                })
            }
            ListGithubAutomationsResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            ListGithubAutomationsResult::Unknown => {
                Err(anyhow!("Unknown response while listing GitHub automations"))
            }
        }
    }

    #[cfg(feature = "github_automations")]
    async fn upsert_github_automation(
        &self,
        workspace_uid: String,
        input: GithubAutomationInput,
    ) -> Result<UpsertGithubAutomationOutcome> {
        use warp_graphql::mutations::upsert_github_automation::{
            GithubAutomationActionInput, GithubAutomationTriggerInput, UpsertGithubAutomation,
            UpsertGithubAutomationInput, UpsertGithubAutomationResult,
            UpsertGithubAutomationVariables,
        };

        let GithubAutomationInput {
            id,
            name,
            enabled,
            trigger,
            action,
        } = input;

        let variables = UpsertGithubAutomationVariables {
            request_context: get_request_context(),
            input: UpsertGithubAutomationInput {
                id: id.map(cynic::Id::new),
                workspace_uid: cynic::Id::new(workspace_uid),
                name,
                enabled,
                trigger: GithubAutomationTriggerInput {
                    event_type: trigger.event_type.to_gql_input(),
                    repo_filter: trigger.repo_filter,
                    branch_pattern: trigger.branch_pattern,
                    comment_phrase: trigger.comment_phrase,
                },
                action: GithubAutomationActionInput {
                    action_type: action.action_type.to_gql_input(),
                    prompt: action.prompt,
                    skill: action.skill,
                    harness: action.harness,
                    model_id: action.model_id,
                },
            },
        };
        let operation = UpsertGithubAutomation::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.upsert_github_automation {
            UpsertGithubAutomationResult::UpsertGithubAutomationOutput(output) => {
                Ok(UpsertGithubAutomationOutcome {
                    automation: output.automation.into(),
                    hook_key: output.hook_key,
                })
            }
            UpsertGithubAutomationResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            UpsertGithubAutomationResult::Unknown => {
                Err(anyhow!("Unknown response while saving GitHub automation"))
            }
        }
    }

    #[cfg(feature = "github_automations")]
    async fn remove_github_automation(&self, workspace_uid: String, id: String) -> Result<()> {
        use warp_graphql::mutations::remove_github_automation::{
            RemoveGithubAutomation, RemoveGithubAutomationInput, RemoveGithubAutomationResult,
            RemoveGithubAutomationVariables,
        };

        let variables = RemoveGithubAutomationVariables {
            request_context: get_request_context(),
            input: RemoveGithubAutomationInput {
                workspace_uid: cynic::Id::new(workspace_uid),
                id: cynic::Id::new(id),
            },
        };
        let operation = RemoveGithubAutomation::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.remove_github_automation {
            RemoveGithubAutomationResult::RemoveGithubAutomationOutput(output) => {
                if output.success {
                    Ok(())
                } else {
                    Err(anyhow!("Server declined to remove the GitHub automation"))
                }
            }
            RemoveGithubAutomationResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            RemoveGithubAutomationResult::Unknown => {
                Err(anyhow!("Unknown response while removing GitHub automation"))
            }
        }
    }

    #[cfg(feature = "github_automations")]
    async fn set_github_provider_key(
        &self,
        workspace_uid: String,
        provider: String,
        key: String,
    ) -> Result<GithubProviderKey> {
        use warp_graphql::mutations::set_github_provider_key::{
            SetGithubProviderKey, SetGithubProviderKeyInput, SetGithubProviderKeyResult,
            SetGithubProviderKeyVariables,
        };

        let variables = SetGithubProviderKeyVariables {
            request_context: get_request_context(),
            input: SetGithubProviderKeyInput {
                workspace_uid: cynic::Id::new(workspace_uid),
                provider,
                key,
            },
        };
        let operation = SetGithubProviderKey::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.set_github_provider_key {
            SetGithubProviderKeyResult::SetGithubProviderKeyOutput(output) => {
                Ok(output.provider_key.into())
            }
            SetGithubProviderKeyResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            SetGithubProviderKeyResult::Unknown => Err(anyhow!(
                "Unknown response while setting GitHub provider key"
            )),
        }
    }

    #[cfg(feature = "github_automations")]
    async fn remove_github_provider_key(
        &self,
        workspace_uid: String,
        provider: String,
    ) -> Result<()> {
        use warp_graphql::mutations::remove_github_provider_key::{
            RemoveGithubProviderKey, RemoveGithubProviderKeyInput, RemoveGithubProviderKeyResult,
            RemoveGithubProviderKeyVariables,
        };

        let variables = RemoveGithubProviderKeyVariables {
            request_context: get_request_context(),
            input: RemoveGithubProviderKeyInput {
                workspace_uid: cynic::Id::new(workspace_uid),
                provider,
            },
        };
        let operation = RemoveGithubProviderKey::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.remove_github_provider_key {
            RemoveGithubProviderKeyResult::RemoveGithubProviderKeyOutput(output) => {
                if output.success {
                    Ok(())
                } else {
                    Err(anyhow!("Server declined to remove the GitHub provider key"))
                }
            }
            RemoveGithubProviderKeyResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            RemoveGithubProviderKeyResult::Unknown => Err(anyhow!(
                "Unknown response while removing GitHub provider key"
            )),
        }
    }
}
