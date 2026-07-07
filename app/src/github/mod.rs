//! Client-side GitHub connection state and token provisioning.
//!
//! [`GithubConnection`] is a singleton entity that mirrors the user's
//! server-mediated GitHub connection (from `get_user_github_info()`), refreshed
//! whenever [`GitHubAuthEvent::AuthCompleted`] fires. It also owns an in-memory
//! GitHub-token cache and hands out a [`github_client::TokenProvider`] that the
//! [`github_client::GithubClient`] uses to authenticate direct api.github.com
//! calls.
//!
//! Only the connection *state* lives on the entity (so UI can read it on the
//! event loop); token fetching is pure-async and goes through a shared cache so
//! the provider can run outside the entity system.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::lock::Mutex;
use github_client::{GithubToken, TokenProvider};
use warp_core::features::FeatureFlag;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::ai::ambient_agents::github_auth_notifier::{GitHubAuthEvent, GitHubAuthNotifier};
use crate::server::server_api::integrations::IntegrationsClient;
use crate::server::server_api::ServerApiProvider;

/// A repo the connected user can see through the GitHub App.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledRepo {
    pub owner: String,
    pub repo: String,
    pub is_public: bool,
    /// Whether the repo is enabled for ambient agents (it belongs to a
    /// workspace-claimed installation). User-driven agents are scoped by the
    /// user's own GitHub access instead.
    pub automation_enabled: bool,
}

impl InstalledRepo {
    /// `owner/repo`, matching GitHub's `full_name`.
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

/// Snapshot of the user's GitHub connection, as reported by the backend.
#[derive(Debug, Clone, Default)]
pub struct GithubConnectionState {
    /// Whether the backend reports a connected GitHub account.
    pub connected: bool,
    /// The connected GitHub username, when known.
    pub username: Option<String>,
    /// Repos the installation can access.
    pub installed_repos: Vec<InstalledRepo>,
    /// Link to install/manage the GitHub App.
    pub app_install_link: Option<String>,
    /// Auth URL to start the connect flow (present when auth is required).
    pub auth_url: Option<String>,
    /// Whether a refresh is currently in flight.
    pub is_loading: bool,
    /// Human-readable error from the last failed refresh, if any.
    pub load_error: Option<String>,
}

impl GithubConnectionState {
    /// Whether `owner/repo` (case-insensitive) is in the installed set.
    pub fn is_repo_installed(&self, owner: &str, repo: &str) -> bool {
        self.installed_repos
            .iter()
            .any(|r| r.owner.eq_ignore_ascii_case(owner) && r.repo.eq_ignore_ascii_case(repo))
    }
}

/// The in-memory GitHub token cache, shared between [`GithubConnection`] and the
/// [`GithubTokenProvider`] it hands out.
#[derive(Default)]
struct TokenCache {
    token: Option<GithubToken>,
}

/// Skew applied when deciding whether a cached token is still usable, so we
/// refresh slightly before the hard expiry.
fn token_expiry_skew() -> chrono::Duration {
    chrono::Duration::seconds(60)
}

/// Singleton mirroring the user's GitHub connection and vending tokens.
pub struct GithubConnection {
    state: GithubConnectionState,
    /// Shared with the [`GithubTokenProvider`]; guarded by an async mutex so the
    /// provider can await it off the event loop.
    token_cache: Arc<Mutex<TokenCache>>,
}

impl Entity for GithubConnection {
    type Event = GithubConnectionEvent;
}

impl SingletonEntity for GithubConnection {}

/// Events emitted when the connection state changes.
#[derive(Debug, Clone)]
pub enum GithubConnectionEvent {
    /// The connection state changed; UI should re-read [`GithubConnection::state`].
    StateChanged,
}

impl GithubConnection {
    /// Construct the singleton, seeding from any persisted token and kicking off
    /// an initial refresh. Subscribes to [`GitHubAuthNotifier`] so a completed
    /// auth flow re-fetches the connection state.
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        // Refetch connection info whenever GitHub auth completes.
        ctx.subscribe_to_model(
            &GitHubAuthNotifier::handle(ctx),
            |me, _, event, ctx| match event {
                GitHubAuthEvent::AuthCompleted => {
                    // Auth changed: drop any cached token and refresh state.
                    me.invalidate_cached_token(ctx);
                    me.refresh(ctx);
                }
            },
        );

        let mut connection = Self {
            state: GithubConnectionState::default(),
            token_cache: Arc::new(Mutex::new(TokenCache::default())),
        };

        // Only do network work when the feature is enabled.
        if FeatureFlag::GithubIntegration.is_enabled() {
            connection.refresh(ctx);
        }

        connection
    }

    /// Current connection snapshot.
    pub fn state(&self) -> &GithubConnectionState {
        &self.state
    }

    /// A [`TokenProvider`] backed by this connection's shared token cache.
    ///
    /// The returned provider can be handed to a [`github_client::GithubClient`]
    /// and used entirely off the event loop.
    pub fn token_provider(&self, ctx: &mut ModelContext<Self>) -> Arc<GithubTokenProvider> {
        let integrations_client = ServerApiProvider::handle(ctx)
            .as_ref(ctx)
            .get_integrations_client();
        Arc::new(GithubTokenProvider {
            cache: self.token_cache.clone(),
            integrations_client,
        })
    }

    /// Clear the cached token so the next request re-mints one.
    fn invalidate_cached_token(&self, ctx: &mut ModelContext<Self>) {
        let cache = self.token_cache.clone();
        // Clearing the cache is cheap and order-independent; run it on the
        // async runtime and ignore the (unit) result.
        ctx.spawn(
            async move {
                cache.lock().await.token = None;
            },
            |_, _, _| {},
        );
    }

    /// Refresh the connection state from `get_user_github_info()`.
    pub fn refresh(&mut self, ctx: &mut ModelContext<Self>) {
        if self.state.is_loading {
            return;
        }
        self.state.is_loading = true;
        self.state.load_error = None;
        ctx.emit(GithubConnectionEvent::StateChanged);

        let integrations_client = ServerApiProvider::handle(ctx)
            .as_ref(ctx)
            .get_integrations_client();
        ctx.spawn(
            async move { integrations_client.get_user_github_info().await },
            |me, result, ctx| {
                me.state.is_loading = false;
                use warp_graphql::queries::user_github_info::UserGithubInfoResult;
                match result {
                    Ok(UserGithubInfoResult::GithubConnectedOutput(info)) => {
                        me.state.connected = true;
                        me.state.username = info.username;
                        me.state.installed_repos = info
                            .installed_repos
                            .into_iter()
                            .map(|r| InstalledRepo {
                                owner: r.owner,
                                repo: r.repo,
                                is_public: r.is_public,
                                automation_enabled: r.automation_enabled.unwrap_or(false),
                            })
                            .collect();
                        // The schema field is non-null; the backend sends ""
                        // when there is no usable install link (e.g. no App
                        // slug configured).
                        me.state.app_install_link =
                            Some(info.app_install_link).filter(|link| !link.is_empty());
                        me.state.auth_url = None;
                    }
                    Ok(UserGithubInfoResult::GithubAuthRequiredOutput(auth)) => {
                        me.state.connected = false;
                        me.state.username = None;
                        me.state.installed_repos.clear();
                        me.state.app_install_link =
                            Some(auth.app_install_link).filter(|link| !link.is_empty());
                        me.state.auth_url = Some(auth.auth_url);
                    }
                    Ok(UserGithubInfoResult::Unknown) => {
                        me.state.load_error =
                            Some("Unexpected response from GitHub info.".to_string());
                    }
                    Err(err) => {
                        log::debug!("GithubConnection: refresh failed: {err:#}");
                        me.state.load_error =
                            Some("Couldn't load GitHub connection status.".to_string());
                    }
                }
                ctx.emit(GithubConnectionEvent::StateChanged);
            },
        );
    }
}

/// [`TokenProvider`] implementation vended by [`GithubConnection`].
///
/// Caches the last token in the shared [`TokenCache`] and refreshes via the
/// backend `get_github_token()` endpoint when the cache is empty or expired.
pub struct GithubTokenProvider {
    cache: Arc<Mutex<TokenCache>>,
    integrations_client: Arc<dyn IntegrationsClient>,
}

#[async_trait]
impl TokenProvider for GithubTokenProvider {
    async fn token(&self) -> anyhow::Result<GithubToken> {
        // TODO(G4): defense-in-depth governance would gate token acquisition
        // here (mirroring McpGovernance spawn gating). Authoritative
        // enforcement is server-side at token minting.
        {
            let cache = self.cache.lock().await;
            if let Some(token) = &cache.token {
                if !token.is_expired_with_skew(token_expiry_skew()) {
                    return Ok(token.clone());
                }
            }
        }

        let response = self.integrations_client.get_github_token().await?;
        let expires_at = response.expires_at.as_deref().and_then(parse_expiry);
        let token = GithubToken {
            token: response.token,
            expires_at,
            installation_id: response.installation_id,
        };

        let mut cache = self.cache.lock().await;
        cache.token = Some(token.clone());
        Ok(token)
    }

    async fn invalidate(&self) {
        self.cache.lock().await.token = None;
    }
}

/// Parse an RFC3339 expiry timestamp; returns `None` on malformed input (the
/// token is then treated as non-expiring, and the strict server-side expiry
/// still applies on the next 401).
fn parse_expiry(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(feature = "github_automations")]
pub mod automations;
pub mod pr_review_comments;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
