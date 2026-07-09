use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

use crate::api::queries::search_marketplace::MarketplaceSourceKind;

// Records that the caller installed a marketplace entry, scoped to a team
// workspace. Powers the per-team install-count popularity leaderboard. Served
// by the Bang backend (src/graphql/marketplacePlugin.js).

#[derive(cynic::QueryVariables, Debug)]
pub struct ReportMarketplaceInstallVariables {
    pub input: ReportMarketplaceInstallInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct ReportMarketplaceInstallInput {
    pub entry_id: cynic::Id,
    pub source: MarketplaceSourceKind,
    pub title: Option<String>,
    pub workspace_id: Option<cynic::Id>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "ReportMarketplaceInstallVariables")]
pub struct ReportMarketplaceInstall {
    #[arguments(input: $input, requestContext: $request_context)]
    pub report_marketplace_install: ReportMarketplaceInstallResult,
}
crate::client::define_operation! {
    report_marketplace_install(ReportMarketplaceInstallVariables) -> ReportMarketplaceInstall;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum ReportMarketplaceInstallResult {
    ReportMarketplaceInstallOutput(ReportMarketplaceInstallOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ReportMarketplaceInstallOutput {
    pub response_context: ResponseContext,
    pub success: bool,
}
