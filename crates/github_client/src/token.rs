//! Token provisioning for [`crate::GithubClient`].
//!
//! The client never mints GitHub tokens itself. Instead it delegates to a
//! [`TokenProvider`] (implemented in the `app` crate by `GithubConnection`,
//! which fetches short-lived user-to-server tokens from the Warp backend via
//! `GET /api/v1/github/token`). This keeps GitHub App credentials out of the
//! client binary and lets token refresh + audit logging live server-side.

use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

/// A short-lived GitHub token plus the metadata the client needs to decide
/// when to treat it as expired and which installation it belongs to.
#[derive(Clone, PartialEq, Eq)]
pub struct GithubToken {
    /// The bearer token to send in the `Authorization` header.
    pub token: String,
    /// Absolute expiry. The client treats expiry strictly and will request a
    /// fresh token once this passes.
    pub expires_at: Option<DateTime<Utc>>,
    /// The GitHub App installation this token was minted for, if known.
    pub installation_id: Option<u64>,
}

impl GithubToken {
    /// Whether the token is expired (or will expire within `skew`).
    ///
    /// A token with no `expires_at` is treated as never-expiring here; callers
    /// that want stricter behavior should set an expiry.
    pub fn is_expired_with_skew(&self, skew: chrono::Duration) -> bool {
        match self.expires_at {
            Some(expires_at) => Utc::now() + skew >= expires_at,
            None => false,
        }
    }
}

// Redact the token when debugging so it never lands in logs.
impl fmt::Debug for GithubToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GithubToken")
            .field("token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .field("installation_id", &self.installation_id)
            .finish()
    }
}

/// Supplies (and invalidates) GitHub tokens for the client.
///
/// Implementations are expected to cache tokens and refresh them as needed.
/// [`invalidate`](TokenProvider::invalidate) is called by the client after a
/// `401` so the next [`token`](TokenProvider::token) call fetches a fresh one.
#[async_trait]
pub trait TokenProvider: Send + Sync {
    /// Return a currently-valid token, refreshing if necessary.
    async fn token(&self) -> anyhow::Result<GithubToken>;

    /// Drop any cached token so the next `token()` call re-fetches.
    async fn invalidate(&self);
}
