use anyhow::{anyhow, Result};
use async_trait::async_trait;
use cynic::QueryBuilder;
#[cfg(test)]
use mockall::automock;
pub use warp_graphql::queries::search_marketplace::{
    MarketplaceEntryKind, MarketplaceSearchEntry, MarketplaceSourceKind,
};
use warp_graphql::queries::search_marketplace::{
    SearchMarketplace, SearchMarketplaceInput, SearchMarketplaceResult, SearchMarketplaceVariables,
};

use super::ServerApi;
use crate::server::graphql::{get_request_context, get_user_facing_error_message};

/// Client for the marketplace directory search op (Bang backend
/// `SearchMarketplace`): the user's org manifests, the official MCP registry,
/// and Open VSX, normalized into one entry shape.
#[cfg_attr(test, automock)]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait MarketplaceClient: 'static + Send + Sync {
    async fn search_marketplace(
        &self,
        source: MarketplaceSourceKind,
        query: Option<String>,
    ) -> Result<Vec<MarketplaceSearchEntry>>;
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
}
