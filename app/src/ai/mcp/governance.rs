//! Org-level MCP governance.
//!
//! Workspace admins on entitled tiers can restrict which MCP servers members
//! may install and run via `WorkspaceSettings.mcpGovernanceSettings`
//! (`DISABLE` kill switch, `ENABLE_ALL`, or an `ALLOWLIST`). This module
//! computes the single *effective* policy for the current user and exposes
//! the decision helpers consulted by every install/spawn choke point.
//!
//! ## Most-restrictive-wins
//!
//! A user can belong to multiple workspaces/teams. Unlike most team-scoped
//! settings (which follow `UserWorkspaces::current_team()`), governance is a
//! *security* policy: an admin who disables MCP expects that to hold no
//! matter which team the user currently has selected. We therefore combine
//! the governance settings of ALL workspaces the user belongs to, and the
//! most restrictive answer wins:
//!
//! * `DISABLE` anywhere wins over everything.
//! * `ALLOWLIST` combines by intersection: a candidate must be allowed by
//!   every governed workspace that chose allowlisting.
//! * `allow_file_based_servers` is AND-ed across governed workspaces.
//! * Workspaces whose tier lacks `marketplace_policy.governance_controls_enabled`
//!   are ignored (entitlement-on-tier pattern), as are workspaces with no
//!   `mcp_governance_settings` (self-managed).
//! * A user with no workspaces/teams (solo) has no governed workspaces and
//!   resolves to `ENABLE_ALL` with untouched code paths.
//!
//! ## Startup / offline enforcement
//!
//! The resolved policy is snapshotted to SQLite whenever it changes and
//! loaded *before* any MCP autostart, so a cached `DISABLE` is enforced
//! offline and before the first `GetWorkspacesMetadataForUser` response. No
//! cached snapshot + no teams ⇒ solo behavior. The in-memory policy is only
//! recomputed from fresh server data (`UserWorkspacesEvent::TeamsChanged`),
//! never from locally cached workspace stubs.
//!
//! ## Cross-repo canonical hash contract (M2)
//!
//! `CANONICAL_HASH` allowlist entries match on
//! `sha256_hex(JSON.stringify(kindTaggedArray))` where the canonical form is
//! `["stdio", command, [args...], [sortedEnvKeyNames...]]` for stdio servers,
//! `["remote", normalizedUrl]` for remote servers (lowercase scheme/host,
//! default ports stripped, trailing path slashes stripped, fragment dropped,
//! query kept verbatim), and `["plugin", normalizedBundleUrl]` for plugin
//! bundles. Env VALUES are never hashed. The algorithm is defined in
//! harness-backend `src/mcp/identity.js` and MUST stay byte-for-byte
//! identical here when candidate matching lands in M2.
//!
//! All enforcement is gated on [`FeatureFlag::McpGovernance`]; with the flag
//! off, every decision helper returns "allowed" and behavior is unchanged.

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::ai::mcp::ServerOrigin;
use crate::persistence::ModelEvent;
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};
use crate::workspaces::workspace::{McpAllowlistEntry, McpGovernanceMode, Workspace};
use crate::GlobalResourceHandlesProvider;

/// The combined governance mode after resolving across all of the user's
/// workspaces (most restrictive wins).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectiveMcpMode {
    /// Kill switch: no MCP servers may be installed or spawned.
    Disable,
    /// No restrictions.
    EnableAll,
    /// One entry list per governed workspace that selected `ALLOWLIST`; a
    /// candidate must be allowed by EVERY list (intersection semantics).
    ///
    /// TODO(M2): candidate fingerprinting/matching against these entries
    /// (mirroring harness-backend `src/mcp/identity.js`). For M1 the
    /// matcher is stubbed to allow-all, so `Allowlist` currently behaves
    /// like `EnableAll` for spawn/install decisions.
    Allowlist {
        allowlists: Vec<Vec<McpAllowlistEntry>>,
    },
}

/// The effective MCP governance policy for the current user. This is what is
/// snapshotted to SQLite and consulted by all enforcement hooks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EffectiveMcpPolicy {
    pub mode: EffectiveMcpMode,
    /// Whether file-based (config-file detected) servers may run. AND-ed
    /// across governed workspaces; `true` for self-managed users.
    pub allow_file_based_servers: bool,
}

impl Default for EffectiveMcpPolicy {
    fn default() -> Self {
        Self::self_managed()
    }
}

impl EffectiveMcpPolicy {
    /// The permissive policy applied to solo / self-managed users (and
    /// whenever the `McpGovernance` feature flag is off).
    pub fn self_managed() -> Self {
        Self {
            mode: EffectiveMcpMode::EnableAll,
            allow_file_based_servers: true,
        }
    }

    /// Resolves the effective policy across ALL of the user's workspaces.
    /// See the module docs for the most-restrictive-wins rationale.
    pub fn resolve(workspaces: &[Workspace]) -> Self {
        let mut disabled = false;
        let mut allowlists: Vec<Vec<McpAllowlistEntry>> = Vec::new();
        let mut allow_file_based_servers = true;

        for workspace in workspaces {
            // Entitlement lives on the billing tier: governance settings on
            // workspaces whose plan doesn't include governance controls are
            // ignored entirely.
            let entitled = workspace
                .billing_metadata
                .tier
                .marketplace_policy
                .is_some_and(|policy| policy.governance_controls_enabled);
            if !entitled {
                continue;
            }
            // Absent settings ⇒ the workspace is self-managed.
            let Some(settings) = &workspace.settings.mcp_governance_settings else {
                continue;
            };

            match settings.mode {
                McpGovernanceMode::Disable => disabled = true,
                McpGovernanceMode::EnableAll => {}
                McpGovernanceMode::Allowlist => allowlists.push(settings.allowlist.clone()),
            }
            allow_file_based_servers &= settings.allow_file_based_servers;
        }

        let mode = if disabled {
            EffectiveMcpMode::Disable
        } else if !allowlists.is_empty() {
            EffectiveMcpMode::Allowlist { allowlists }
        } else {
            EffectiveMcpMode::EnableAll
        };

        Self {
            mode,
            allow_file_based_servers,
        }
    }

    /// Whether new MCP server installs/imports (gallery installs, Cursor
    /// import, manual adds) are allowed at all.
    pub fn allows_new_installs(&self) -> bool {
        !matches!(self.mode, EffectiveMcpMode::Disable)
    }

    /// Whether a server with the given provenance may be spawned (or kept
    /// running). This is the single decision consulted by every spawn choke
    /// point and by the policy-tightening shutdown pass.
    pub fn allows_spawn(&self, origin: ServerOrigin) -> bool {
        match &self.mode {
            EffectiveMcpMode::Disable => false,
            // TODO(M2): Allowlist candidate matching. M1 stubs allowlist
            // matching to allow-all, so only the file-based gate applies.
            EffectiveMcpMode::EnableAll | EffectiveMcpMode::Allowlist { .. } => {
                if matches!(origin, ServerOrigin::FileBased) {
                    self.allow_file_based_servers
                } else {
                    true
                }
            }
        }
    }

    /// Given the set of currently running servers (uuid + provenance),
    /// returns the servers that are no longer allowed under this policy and
    /// must be shut down. Loosening never produces restarts: this only ever
    /// selects servers to stop.
    pub fn compute_shutdown_set(
        &self,
        running_servers: impl IntoIterator<Item = (Uuid, ServerOrigin)>,
    ) -> Vec<Uuid> {
        running_servers
            .into_iter()
            .filter(|(_, origin)| !self.allows_spawn(*origin))
            .map(|(uuid, _)| uuid)
            .collect()
    }
}

/// Events emitted by [`McpGovernance`].
pub enum McpGovernanceEvent {
    /// The effective policy changed following a workspace-metadata refresh.
    /// Subscribers should re-evaluate running servers / UI affordances.
    PolicyChanged,
}

/// Singleton app model owning the effective MCP governance policy.
pub struct McpGovernance {
    effective_policy: EffectiveMcpPolicy,
}

impl Entity for McpGovernance {
    type Event = McpGovernanceEvent;
}

impl SingletonEntity for McpGovernance {}

impl McpGovernance {
    /// `cached_policy_json` is the SQLite snapshot loaded at startup, applied
    /// BEFORE any MCP autostart so a cached `DISABLE` is enforced offline.
    pub fn new(cached_policy_json: Option<String>, ctx: &mut ModelContext<Self>) -> Self {
        let effective_policy = cached_policy_json
            .as_deref()
            .and_then(
                |json| match serde_json::from_str::<EffectiveMcpPolicy>(json) {
                    Ok(policy) => Some(policy),
                    Err(err) => {
                        log::error!("Failed to parse cached MCP governance policy snapshot: {err}");
                        None
                    }
                },
            )
            // No cached snapshot (fresh install, or pre-governance builds):
            // solo behavior until fresh workspace metadata arrives.
            .unwrap_or_else(EffectiveMcpPolicy::self_managed);

        if FeatureFlag::McpGovernance.is_enabled() {
            // `TeamsChanged` is only emitted from fresh server responses
            // (`on_workspaces_updated`), never from the local workspace
            // cache, so recomputing here never trusts stale/dummy team data.
            ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _, event, ctx| {
                if matches!(event, UserWorkspacesEvent::TeamsChanged) {
                    me.recompute_policy(ctx);
                }
            });
        }

        Self { effective_policy }
    }

    pub fn effective_policy(&self) -> &EffectiveMcpPolicy {
        &self.effective_policy
    }

    /// The policy to enforce right now. Honors the feature flag: when
    /// `McpGovernance` is off this always returns the permissive
    /// self-managed policy, keeping behavior identical to pre-governance
    /// builds.
    pub fn current_policy(app: &AppContext) -> EffectiveMcpPolicy {
        if !FeatureFlag::McpGovernance.is_enabled() {
            return EffectiveMcpPolicy::self_managed();
        }
        McpGovernance::as_ref(app).effective_policy().clone()
    }

    /// Convenience for UI surfaces: whether MCP is fully disabled by org
    /// policy (kill switch).
    pub fn is_disabled_by_org(app: &AppContext) -> bool {
        matches!(Self::current_policy(app).mode, EffectiveMcpMode::Disable)
    }

    fn recompute_policy(&mut self, ctx: &mut ModelContext<Self>) {
        let new_policy = EffectiveMcpPolicy::resolve(UserWorkspaces::as_ref(ctx).workspaces());
        if new_policy == self.effective_policy {
            return;
        }
        log::info!(
            "Effective MCP governance policy changed: {:?} -> {:?}",
            self.effective_policy,
            new_policy
        );
        self.effective_policy = new_policy;
        self.persist_snapshot(ctx);
        ctx.emit(McpGovernanceEvent::PolicyChanged);
        ctx.notify();
    }

    /// Test-only: force a specific effective policy and notify subscribers,
    /// bypassing workspace resolution and snapshot persistence.
    #[cfg(test)]
    pub fn set_policy_for_tests(
        &mut self,
        policy: EffectiveMcpPolicy,
        ctx: &mut ModelContext<Self>,
    ) {
        self.effective_policy = policy;
        ctx.emit(McpGovernanceEvent::PolicyChanged);
        ctx.notify();
    }

    fn persist_snapshot(&self, ctx: &mut ModelContext<Self>) {
        let policy_json = match serde_json::to_string(&self.effective_policy) {
            Ok(json) => json,
            Err(err) => {
                log::error!("Failed to serialize MCP governance policy snapshot: {err}");
                return;
            }
        };
        let global_resource_handles = GlobalResourceHandlesProvider::as_ref(ctx).get();
        if let Some(sender) = &global_resource_handles.model_event_sender {
            if let Err(err) = sender.send(ModelEvent::UpsertMcpGovernancePolicy { policy_json }) {
                log::error!("Failed to save MCP governance policy snapshot to database: {err}");
            }
        }
    }
}

#[cfg(test)]
#[path = "governance_tests.rs"]
mod tests;
