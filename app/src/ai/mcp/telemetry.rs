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
    BlockedInstall,
    /// A server spawn (start, restart, reconnect, or auto-start) was blocked
    /// by org policy.
    BlockedSpawn,
    /// A policy tightening shut down running servers.
    PolicyShutdown { server_count: usize },
}

impl TelemetryEvent for McpGovernanceTelemetryEvent {
    fn name(&self) -> &'static str {
        McpGovernanceTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            Self::BlockedInstall | Self::BlockedSpawn => None,
            Self::PolicyShutdown { server_count } => Some(json!({
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
            Self::BlockedInstall | Self::BlockedSpawn | Self::PolicyShutdown { .. } => false,
        }
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for McpGovernanceTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::BlockedInstall => "MCP Governance Blocked Install",
            Self::BlockedSpawn => "MCP Governance Blocked Spawn",
            Self::PolicyShutdown => "MCP Governance Policy Shutdown",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::BlockedInstall => {
                "An MCP server install or import was blocked by the organization's governance policy"
            }
            Self::BlockedSpawn => {
                "An MCP server spawn was blocked by the organization's governance policy"
            }
            Self::PolicyShutdown => {
                "Running MCP servers were shut down because the organization's governance policy tightened"
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            Self::BlockedInstall | Self::BlockedSpawn | Self::PolicyShutdown => {
                EnablementState::Flag(FeatureFlag::McpGovernance)
            }
        }
    }
}

warp_core::register_telemetry_event!(McpGovernanceTelemetryEvent);
