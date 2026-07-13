use super::search_marketplace::MarketplaceSourceKind;
use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

// Resolves a marketplace plugin's full component contents (MCP configs and the
// bodies of its rule / command / agent / skill files) for installation. Kept
// separate from SearchMarketplace so directory listings stay lightweight; the
// client calls this only when the user installs an entry. Served by the Bang
// backend (src/graphql/marketplacePlugin.js).

#[derive(cynic::QueryVariables, Debug)]
pub struct ResolveMarketplacePluginVariables {
    pub input: ResolveMarketplacePluginInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct ResolveMarketplacePluginInput {
    pub entry_id: cynic::Id,
    pub source: MarketplaceSourceKind,
}

#[derive(cynic::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarketplaceComponentType {
    #[cynic(rename = "AGENT")]
    Agent,
    #[cynic(rename = "COMMAND")]
    Command,
    #[cynic(rename = "HOOK")]
    Hook,
    #[cynic(rename = "MCP_SERVER")]
    McpServer,
    #[cynic(rename = "RULE")]
    Rule,
    #[cynic(rename = "SKILL")]
    Skill,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct MarketplacePluginComponentFile {
    pub component_type: MarketplaceComponentType,
    pub content: String,
    pub name: String,
    pub path: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ResolveMarketplacePluginOutput {
    pub entry_id: cynic::Id,
    pub files: Vec<MarketplacePluginComponentFile>,
    pub mcp_template_json: Option<String>,
    pub response_context: ResponseContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootQuery",
    variables = "ResolveMarketplacePluginVariables"
)]
pub struct ResolveMarketplacePlugin {
    #[arguments(input: $input, requestContext: $request_context)]
    pub resolve_marketplace_plugin: ResolveMarketplacePluginResult,
}
crate::client::define_operation! {
    resolve_marketplace_plugin(ResolveMarketplacePluginVariables) -> ResolveMarketplacePlugin;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum ResolveMarketplacePluginResult {
    ResolveMarketplacePluginOutput(ResolveMarketplacePluginOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}
