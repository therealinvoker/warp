use anyhow::{anyhow, Result};
use async_trait::async_trait;
use cynic::{MutationBuilder, QueryBuilder};
#[cfg(test)]
use mockall::automock;
use warp_graphql::mutations::report_marketplace_install::{
    ReportMarketplaceInstall, ReportMarketplaceInstallInput, ReportMarketplaceInstallResult,
    ReportMarketplaceInstallVariables,
};
pub use warp_graphql::queries::resolve_marketplace_plugin::{
    MarketplaceComponentType, MarketplacePluginComponentFile,
};
use warp_graphql::queries::resolve_marketplace_plugin::{
    ResolveMarketplacePlugin, ResolveMarketplacePluginInput, ResolveMarketplacePluginResult,
    ResolveMarketplacePluginVariables,
};
pub use warp_graphql::queries::search_marketplace::{
    MarketplaceEntryKind, MarketplaceSearchEntry, MarketplaceSourceKind,
};
use warp_graphql::queries::search_marketplace::{
    SearchMarketplace, SearchMarketplaceInput, SearchMarketplaceResult, SearchMarketplaceVariables,
};

use super::ServerApi;
use crate::server::graphql::{get_request_context, get_user_facing_error_message};

/// The fully-resolved contents of a marketplace plugin, fetched on demand at
/// install time (see [`MarketplaceClient::resolve_marketplace_plugin`]).
#[derive(Debug, Clone, Default)]
pub struct ResolvedMarketplacePlugin {
    /// One entry per component file (rule / command / agent / skill body),
    /// grouped by `name`.
    pub files: Vec<MarketplacePluginComponentFile>,
    /// The `{"mcpServers": {...}}` config for the plugin's MCP part, if any.
    pub mcp_template_json: Option<String>,
}

/// Client for the marketplace directory ops (Bang backend): `SearchMarketplace`
/// lists the user's org manifests, the official MCP registry, and Open VSX;
/// `ResolveMarketplacePlugin` fetches a plugin's full component bodies for
/// install; `ReportMarketplaceInstall` records an install for the per-team
/// popularity leaderboard.
#[cfg_attr(test, automock)]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait MarketplaceClient: 'static + Send + Sync {
    async fn search_marketplace(
        &self,
        source: MarketplaceSourceKind,
        query: Option<String>,
    ) -> Result<Vec<MarketplaceSearchEntry>>;

    async fn resolve_marketplace_plugin(
        &self,
        source: MarketplaceSourceKind,
        entry_id: String,
    ) -> Result<ResolvedMarketplacePlugin>;

    async fn report_marketplace_install(
        &self,
        source: MarketplaceSourceKind,
        entry_id: String,
        title: Option<String>,
        workspace_id: Option<String>,
    ) -> Result<()>;
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl MarketplaceClient for ServerApi {
    async fn search_marketplace(
        &self,
        source: MarketplaceSourceKind,
        query: Option<String>,
    ) -> Result<Vec<MarketplaceSearchEntry>> {
        let variables = SearchMarketplaceVariables {
            input: SearchMarketplaceInput { query, source },
            request_context: get_request_context(),
        };
        let operation = SearchMarketplace::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.search_marketplace {
            SearchMarketplaceResult::SearchMarketplaceOutput(output) => Ok(output.entries),
            SearchMarketplaceResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            // Op not implemented by the server (empty `{"data":{}}` reply)
            // decodes as Unknown; surface it as a soft, user-readable error.
            SearchMarketplaceResult::Unknown => Err(anyhow!(
                "Marketplace search is not supported by the server yet."
            )),
        }
    }

    async fn resolve_marketplace_plugin(
        &self,
        source: MarketplaceSourceKind,
        entry_id: String,
    ) -> Result<ResolvedMarketplacePlugin> {
        let variables = ResolveMarketplacePluginVariables {
            input: ResolveMarketplacePluginInput {
                entry_id: cynic::Id::new(entry_id),
                source,
            },
            request_context: get_request_context(),
        };
        let operation = ResolveMarketplacePlugin::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.resolve_marketplace_plugin {
            ResolveMarketplacePluginResult::ResolveMarketplacePluginOutput(output) => {
                Ok(ResolvedMarketplacePlugin {
                    files: output.files,
                    mcp_template_json: output.mcp_template_json,
                })
            }
            ResolveMarketplacePluginResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            ResolveMarketplacePluginResult::Unknown => Err(anyhow!(
                "Marketplace plugin resolution is not supported by the server yet."
            )),
        }
    }

    async fn report_marketplace_install(
        &self,
        source: MarketplaceSourceKind,
        entry_id: String,
        title: Option<String>,
        workspace_id: Option<String>,
    ) -> Result<()> {
        let variables = ReportMarketplaceInstallVariables {
            input: ReportMarketplaceInstallInput {
                entry_id: cynic::Id::new(entry_id),
                source,
                title,
                workspace_id: workspace_id.map(cynic::Id::new),
            },
            request_context: get_request_context(),
        };
        let operation = ReportMarketplaceInstall::build(variables);
        let response = self.send_graphql_request(operation, None).await?;

        match response.report_marketplace_install {
            ReportMarketplaceInstallResult::ReportMarketplaceInstallOutput(_) => Ok(()),
            ReportMarketplaceInstallResult::UserFacingError(error) => {
                Err(anyhow!(get_user_facing_error_message(error)))
            }
            // Unknown (server has no handler) is a benign no-op for a
            // fire-and-forget install-count report.
            ReportMarketplaceInstallResult::Unknown => Ok(()),
        }
    }
}
