use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

// Searches one marketplace directory (the user's org manifests, the official
// MCP registry, or Open VSX) and returns normalized entries. Served by the
// Bang backend (src/graphql/marketplaceSearch.js).

#[derive(cynic::QueryVariables, Debug)]
pub struct SearchMarketplaceVariables {
    pub input: SearchMarketplaceInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct SearchMarketplaceInput {
    pub query: Option<String>,
    pub source: MarketplaceSourceKind,
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarketplaceSourceKind {
    #[cynic(rename = "MCP_REGISTRY")]
    McpRegistry,
    #[cynic(rename = "OPEN_VSX")]
    OpenVsx,
    #[cynic(rename = "ORG")]
    Org,
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarketplaceEntryKind {
    #[cynic(rename = "MCP")]
    Mcp,
    #[cynic(rename = "PLUGIN")]
    Plugin,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct MarketplaceSearchEntry {
    pub bundle_url: Option<String>,
    pub description: String,
    pub entry_id: cynic::Id,
    pub extension_id: Option<String>,
    pub icon_url: Option<String>,
    pub kind: MarketplaceEntryKind,
    pub mcp_template_json: Option<String>,
    pub publisher: Option<String>,
    pub source_label: String,
    pub title: String,
    pub version: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct SearchMarketplaceOutput {
    pub entries: Vec<MarketplaceSearchEntry>,
    pub response_context: ResponseContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "SearchMarketplaceVariables")]
pub struct SearchMarketplace {
    #[arguments(input: $input, requestContext: $request_context)]
    pub search_marketplace: SearchMarketplaceResult,
}
crate::client::define_operation! {
    search_marketplace(SearchMarketplaceVariables) -> SearchMarketplace;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum SearchMarketplaceResult {
    SearchMarketplaceOutput(SearchMarketplaceOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
