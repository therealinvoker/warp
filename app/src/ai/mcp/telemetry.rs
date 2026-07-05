//! Telemetry for org-level MCP governance enforcement.

use serde_json::{json, Value};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::features::FeatureFlag;
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

/// Events recording when MCP governance policy blocks an operation or shuts
/// down running servers. Counts only; no server identities or UGC.
#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub enum McpGovernanceTelemetryEvent {
    /// An install/import was blocked by org policy.
    GovernanceBlockedInstall,
    /// A server spawn (start, restart, reconnect, or auto-start) was blocked
    /// by org policy.
    GovernanceBlockedSpawn,
    /// A policy tightening shut down running servers.
    GovernancePolicyShutdown { server_count: usize },
}

impl TelemetryEvent for McpGovernanceTelemetryEvent {
    fn name(&self) -> &'static str {
        McpGovernanceTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            Self::GovernanceBlockedInstall | Self::GovernanceBlockedSpawn => None,
            Self::GovernancePolicyShutdown { server_count } => Some(json!({
                "server_count": server_count,
            })),
        }
    }

    fn description(&self) -> &'static str {
        McpGovernanceTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        McpGovernanceTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        match self {
            Self::GovernanceBlockedInstall
            | Self::GovernanceBlockedSpawn
            | Self::GovernancePolicyShutdown { .. } => false,
        }
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for McpGovernanceTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::GovernanceBlockedInstall => "MCP Governance Blocked Install",
            Self::GovernanceBlockedSpawn => "MCP Governance Blocked Spawn",
            Self::GovernancePolicyShutdown => "MCP Governance Policy Shutdown",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::GovernanceBlockedInstall => {
                "An MCP server install or import was blocked by the organization's governance policy"
            }
            Self::GovernanceBlockedSpawn => {
                "An MCP server spawn was blocked by the organization's governance policy"
            }
            Self::GovernancePolicyShutdown => {
                "Running MCP servers were shut down because the organization's governance policy tightened"
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            Self::GovernanceBlockedInstall
            | Self::GovernanceBlockedSpawn
            | Self::GovernancePolicyShutdown => EnablementState::Flag(FeatureFlag::McpGovernance),
        }
    }
}

warp_core::register_telemetry_event!(McpGovernanceTelemetryEvent);
