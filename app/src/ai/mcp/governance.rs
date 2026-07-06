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
//! ## Cross-repo canonical hash contract
//!
//! `CANONICAL_HASH` allowlist entries match on
//! `sha256_hex(JSON.stringify(kindTaggedArray))` where the canonical form is
//! `["stdio", command, [args...], [sortedEnvKeyNames...]]` for stdio servers,
//! `["remote", normalizedUrl]` for remote servers (lowercase scheme/host,
//! default ports stripped, trailing path slashes stripped, fragment dropped,
//! query kept verbatim), and `["plugin", normalizedBundleUrl]` for plugin
//! bundles. Env VALUES are never hashed. The algorithm is defined in
//! harness-backend `src/mcp/identity.js` and is mirrored byte-for-byte by
//! [`CandidateSpec::canonical_string`] / [`CandidateSpec::canonical_hash`]
//! below; any change on either side MUST be made on the other (the test
//! vectors in `governance_tests.rs` were generated from `identity.js`).
//!
//! ## Allowlist entry semantics
//!
//! * `REGISTRY_NAME` — exact match on the server/template name.
//! * `GALLERY_TEMPLATE` — UUID equality with the gallery item id.
//! * `ORG_MARKETPLACE_ENTRY` — UUID equality with the org marketplace entry
//!   id (populated once org sources land client-side).
//! * `URL_PATTERN` — matched against the *normalized* remote/bundle URL. A
//!   pattern containing `*` is a wildcard match (`*` matches any run of
//!   characters, including `/`); a pattern without `*` is a prefix match, so
//!   `https://mcp.corp.example.com` allows every endpoint under that host.
//! * `COMMAND_PATTERN` — matched against the stdio command (argv\[0\],
//!   verbatim, no path resolution). A pattern containing `*` is a wildcard
//!   match; a pattern without `*` must match the command *exactly* (prefix
//!   matching on commands would let `npx-evil` ride an `npx` entry).
//! * `CANONICAL_HASH` — case-insensitive equality with the canonical hash.
//!
//! `pinnedVersion` is honored where the candidate's version is known (gallery
//! installs carry their gallery version): if both the entry pin and the
//! candidate version are present they must be equal; an unknown candidate
//! version does not fail the pin.
//!
//! All enforcement is gated on [`FeatureFlag::McpGovernance`]; with the flag
//! off, every decision helper returns "allowed" and behavior is unchanged.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::ai::mcp::parsing::resolve_json;
use crate::ai::mcp::templatable_installation::TemplatableMCPServerInstallation;
use crate::ai::mcp::{ServerOrigin, TemplatableMCPServer};
use crate::persistence::ModelEvent;
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};
use crate::workspaces::workspace::{
    McpAllowlistEntry, McpAllowlistEntryKind, McpGovernanceMode, Workspace,
};
use crate::GlobalResourceHandlesProvider;

/// Transport-level identity of a candidate MCP server, in the canonical form
/// shared with harness-backend `src/mcp/identity.js`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CandidateSpec {
    /// A stdio (command-launched) server. `env_keys` holds env var NAMES
    /// only — values hold secrets and are never part of the identity.
    Stdio {
        command: String,
        args: Vec<String>,
        env_keys: Vec<String>,
    },
    /// A remote (SSE/HTTP) server, keyed by its URL.
    Remote { url: String },
    /// A plugin bundle, keyed by its bundle URL. Part of the cross-repo
    /// identity contract; production code starts constructing it once plugin
    /// import fingerprinting lands (today only the contract tests build it).
    #[allow(dead_code)]
    Plugin { bundle_url: String },
}

impl CandidateSpec {
    /// The canonical (pre-hash) identity string — CROSS-REPO CONTRACT.
    ///
    /// Must stay byte-for-byte identical to `canonicalString` in
    /// harness-backend `src/mcp/identity.js`: the JSON serialization (no
    /// whitespace, `JSON.stringify` semantics) of a kind-tagged array. JSON
    /// array encoding makes the hash immune to delimiter injection between
    /// components.
    pub fn canonical_string(&self) -> String {
        let value = match self {
            CandidateSpec::Stdio {
                command,
                args,
                env_keys,
            } => {
                let mut sorted_keys = env_keys.clone();
                sorted_keys.sort();
                serde_json::json!(["stdio", command, args, sorted_keys])
            }
            CandidateSpec::Remote { url } => {
                serde_json::json!(["remote", normalize_url(url)])
            }
            CandidateSpec::Plugin { bundle_url } => {
                serde_json::json!(["plugin", normalize_url(bundle_url)])
            }
        };
        // Serializing a just-built Value never fails.
        serde_json::to_string(&value).unwrap_or_default()
    }

    /// sha256 hex digest (lowercase) of [`Self::canonical_string`].
    pub fn canonical_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.canonical_string().as_bytes());
        hex::encode(hasher.finalize())
    }
}

/// Normalizes a server/bundle URL per the cross-repo contract: lowercase
/// scheme/host, default ports stripped, trailing path slashes stripped (path
/// `/` becomes empty), fragment dropped, query kept verbatim. Non-parseable
/// input is returned trimmed as-is so hashing still succeeds
/// deterministically. Mirrors `normalizeUrl` in harness-backend
/// `src/mcp/identity.js`.
pub fn normalize_url(raw: &str) -> String {
    let trimmed = raw.trim();
    let Ok(url) = url::Url::parse(trimmed) else {
        return trimmed.to_string();
    };
    let host = url.host_str().unwrap_or_default();
    // `Url::port()` is `None` when the port is the scheme default, matching
    // the JS `URL.host` behavior of omitting default ports.
    let host_port = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    let path = url.path().trim_end_matches('/');
    let search = url
        .query()
        .filter(|query| !query.is_empty())
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    format!("{}://{host_port}{path}{search}", url.scheme())
}

/// Everything the allowlist matcher may key on for one candidate MCP server.
/// Fields are `None` when that identity facet is unknown for the candidate
/// (e.g. no gallery provenance, or an unparseable template).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerCandidate {
    /// The server/template display name (`REGISTRY_NAME` matching).
    pub name: Option<String>,
    /// The gallery item id for gallery installs (`GALLERY_TEMPLATE`).
    pub gallery_template_id: Option<Uuid>,
    /// The org marketplace entry id (`ORG_MARKETPLACE_ENTRY`). Not yet
    /// populated: the client doesn't carry org-source provenance ids.
    pub org_marketplace_entry_id: Option<Uuid>,
    /// The candidate's version where known (gallery version today).
    pub version: Option<String>,
    /// Parsed transport identity for URL/command/hash matching.
    pub spec: Option<CandidateSpec>,
}

impl ServerCandidate {
    /// Candidate for a template that isn't installed yet. Template variables
    /// are left unresolved (`{{VAR}}` placeholders stay in place), so
    /// `CANONICAL_HASH` entries generally only match fully-concrete
    /// templates; name/gallery/pattern matching is unaffected.
    pub fn from_template(template: &TemplatableMCPServer) -> Self {
        Self::from_template_and_json(template, &template.template.json)
    }

    /// Candidate for an installation, with template variables resolved to
    /// their concrete values.
    pub fn from_installation(installation: &TemplatableMCPServerInstallation) -> Self {
        Self::from_template_and_json(installation.templatable_mcp_server(), &resolve_json(installation))
    }

    fn from_template_and_json(template: &TemplatableMCPServer, config_json: &str) -> Self {
        Self {
            name: Some(template.name.clone()),
            gallery_template_id: template
                .gallery_data
                .as_ref()
                .map(|gallery_data| gallery_data.gallery_item_id),
            org_marketplace_entry_id: None,
            version: template
                .gallery_data
                .as_ref()
                .map(|gallery_data| gallery_data.version.to_string()),
            spec: spec_from_config_json(config_json),
        }
    }
}

/// Parses a single-server MCP config JSON (the shape stored in
/// `JsonTemplate::json`) into its transport identity. Returns `None` when the
/// JSON is malformed or contains no recognizable server.
fn spec_from_config_json(config_json: &str) -> Option<CandidateSpec> {
    use cloud_object_models::{JSONMCPServer, JSONTransportType};

    let trimmed = config_json.trim();
    // Some docs omit the outer braces; mirror the permissive parsers.
    let wrapped = if trimmed.starts_with('{') {
        trimmed.to_string()
    } else {
        format!("{{{trimmed}}}")
    };
    let config: serde_json::Value = serde_json::from_str(&wrapped).ok()?;
    let servers = TemplatableMCPServer::find_template_map(config).ok()?;
    // Stored templates contain exactly one server; take it.
    let (_, server_json) = servers.into_iter().next()?;
    let server: JSONMCPServer = serde_json::from_value(server_json).ok()?;
    Some(match server.transport_type {
        JSONTransportType::CLIServer {
            command, args, env, ..
        } => {
            let mut env_keys: Vec<String> = env.into_keys().collect();
            env_keys.sort();
            CandidateSpec::Stdio {
                command,
                args,
                env_keys,
            }
        }
        JSONTransportType::SSEServer { url, .. } => CandidateSpec::Remote { url },
    })
}

/// `*`-only wildcard matcher (no `?`, no character classes). `*` matches any
/// run of characters, including `/`.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut star_text = 0usize;
    while t < text.len() {
        if p < pattern.len() && pattern[p] != '*' && pattern[p] == text[t] {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            star_text = t;
            p += 1;
        } else if let Some(star_pos) = star {
            p = star_pos + 1;
            star_text += 1;
            t = star_text;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

/// Whether one allowlist entry matches the candidate. See the module docs
/// for the per-kind semantics.
fn entry_matches(entry: &McpAllowlistEntry, candidate: &ServerCandidate) -> bool {
    let value = entry.value.trim();
    let kind_matches = match entry.kind {
        McpAllowlistEntryKind::RegistryName => {
            candidate.name.as_deref().is_some_and(|name| name == value)
        }
        McpAllowlistEntryKind::GalleryTemplate => matches!(
            (Uuid::parse_str(value), candidate.gallery_template_id),
            (Ok(entry_id), Some(candidate_id)) if entry_id == candidate_id
        ),
        McpAllowlistEntryKind::OrgMarketplaceEntry => matches!(
            (Uuid::parse_str(value), candidate.org_marketplace_entry_id),
            (Ok(entry_id), Some(candidate_id)) if entry_id == candidate_id
        ),
        McpAllowlistEntryKind::UrlPattern => match &candidate.spec {
            Some(CandidateSpec::Remote { url }) | Some(CandidateSpec::Plugin { bundle_url: url }) => {
                let normalized = normalize_url(url);
                if value.contains('*') {
                    wildcard_match(value, &normalized)
                } else {
                    normalized.starts_with(value)
                }
            }
            _ => false,
        },
        McpAllowlistEntryKind::CommandPattern => match &candidate.spec {
            Some(CandidateSpec::Stdio { command, .. }) => {
                if value.contains('*') {
                    wildcard_match(value, command)
                } else {
                    command == value
                }
            }
            _ => false,
        },
        McpAllowlistEntryKind::CanonicalHash => candidate
            .spec
            .as_ref()
            .is_some_and(|spec| spec.canonical_hash().eq_ignore_ascii_case(value)),
    };

    kind_matches && pinned_version_matches(entry, candidate)
}

/// `pinnedVersion` is honored where the candidate's version is known: both
/// present ⇒ must be equal; candidate version unknown ⇒ the pin does not
/// constrain.
fn pinned_version_matches(entry: &McpAllowlistEntry, candidate: &ServerCandidate) -> bool {
    match (&entry.pinned_version, &candidate.version) {
        (Some(pinned), Some(version)) => pinned.trim() == version.trim(),
        _ => true,
    }
}

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
    /// import, manual adds) are allowed at all. Coarse gate consulted by
    /// add/import affordances; candidate-specific installs go through
    /// [`Self::allows_install`].
    pub fn allows_new_installs(&self) -> bool {
        !matches!(self.mode, EffectiveMcpMode::Disable)
    }

    /// Whether the given candidate passes every governed workspace's
    /// allowlist (intersection semantics). `EnableAll` trivially allows;
    /// `Disable` trivially denies.
    fn allowlists_allow(&self, candidate: &ServerCandidate) -> bool {
        match &self.mode {
            EffectiveMcpMode::Disable => false,
            EffectiveMcpMode::EnableAll => true,
            EffectiveMcpMode::Allowlist { allowlists } => allowlists.iter().all(|allowlist| {
                allowlist
                    .iter()
                    .any(|entry| entry_matches(entry, candidate))
            }),
        }
    }

    /// Whether this specific candidate may be installed/imported.
    pub fn allows_install(&self, candidate: &ServerCandidate) -> bool {
        self.allows_new_installs() && self.allowlists_allow(candidate)
    }

    /// Whether a server with the given provenance and identity may be
    /// spawned (or kept running). This is the single decision consulted by
    /// every spawn choke point and by the policy-tightening shutdown pass.
    ///
    /// `candidate == None` means no fingerprint could be derived (e.g.
    /// untracked ephemeral servers): only the coarse mode/file-based gates
    /// apply then, and the backend remains the authoritative allowlist
    /// enforcer for such servers.
    pub fn allows_spawn(&self, origin: ServerOrigin, candidate: Option<&ServerCandidate>) -> bool {
        match &self.mode {
            EffectiveMcpMode::Disable => false,
            EffectiveMcpMode::EnableAll | EffectiveMcpMode::Allowlist { .. } => {
                if matches!(origin, ServerOrigin::FileBased) && !self.allow_file_based_servers {
                    return false;
                }
                match candidate {
                    Some(candidate) => self.allowlists_allow(candidate),
                    None => true,
                }
            }
        }
    }

    /// Given the set of currently running servers (uuid + provenance +
    /// identity), returns the servers that are no longer allowed under this
    /// policy and must be shut down. Loosening never produces restarts: this
    /// only ever selects servers to stop.
    pub fn compute_shutdown_set(
        &self,
        running_servers: impl IntoIterator<Item = (Uuid, ServerOrigin, Option<ServerCandidate>)>,
    ) -> Vec<Uuid> {
        running_servers
            .into_iter()
            .filter(|(_, origin, candidate)| !self.allows_spawn(*origin, candidate.as_ref()))
            .map(|(uuid, _, _)| uuid)
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
