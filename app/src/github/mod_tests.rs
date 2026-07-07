//! Unit tests for GitHub connection state and token provisioning logic.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use chrono::Utc;
use futures::executor::block_on;
use futures::lock::Mutex;
use github_client::TokenProvider;

use super::*;
use crate::server::server_api::integrations::{GithubTokenResponse, MockIntegrationsClient};

#[test]
fn installed_repo_full_name() {
    let repo = InstalledRepo {
        owner: "warpdotdev".to_string(),
        repo: "warp".to_string(),
        is_public: true,
        automation_enabled: false,
    };
    assert_eq!(repo.full_name(), "warpdotdev/warp");
}

#[test]
fn is_repo_installed_is_case_insensitive() {
    let state = GithubConnectionState {
        connected: true,
        installed_repos: vec![InstalledRepo {
            owner: "WarpDotDev".to_string(),
            repo: "Warp".to_string(),
            is_public: false,
            automation_enabled: true,
        }],
        ..Default::default()
    };
    assert!(state.is_repo_installed("warpdotdev", "warp"));
    assert!(state.is_repo_installed("WARPDOTDEV", "WARP"));
    assert!(!state.is_repo_installed("other", "warp"));
}

#[test]
fn parse_expiry_handles_rfc3339_and_garbage() {
    assert!(parse_expiry("2030-01-01T00:00:00Z").is_some());
    assert!(parse_expiry("not-a-date").is_none());
}

/// Build a token provider around a mocked integrations client.
fn provider_with(client: MockIntegrationsClient) -> Arc<GithubTokenProvider> {
    Arc::new(GithubTokenProvider {
        cache: Arc::new(Mutex::new(TokenCache::default())),
        integrations_client: Arc::new(client),
    })
}

#[test]
fn token_provider_fetches_and_caches() {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = calls.clone();
    let mut client = MockIntegrationsClient::new();
    client.expect_get_github_token().returning(move || {
        calls_clone.fetch_add(1, Ordering::SeqCst);
        Ok(GithubTokenResponse {
            token: "gho_abc".to_string(),
            expires_at: Some((Utc::now() + chrono::Duration::minutes(30)).to_rfc3339()),
            installation_id: Some(7),
        })
    });

    let provider = provider_with(client);

    let token = block_on(provider.token()).unwrap();
    assert_eq!(token.token, "gho_abc");
    assert_eq!(token.installation_id, Some(7));

    // Second call is served from cache (no additional fetch).
    let token2 = block_on(provider.token()).unwrap();
    assert_eq!(token2.token, "gho_abc");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn token_provider_refetches_after_invalidate() {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = calls.clone();
    let mut client = MockIntegrationsClient::new();
    client.expect_get_github_token().returning(move || {
        let n = calls_clone.fetch_add(1, Ordering::SeqCst);
        Ok(GithubTokenResponse {
            token: format!("gho_{n}"),
            expires_at: Some((Utc::now() + chrono::Duration::minutes(30)).to_rfc3339()),
            installation_id: None,
        })
    });

    let provider = provider_with(client);

    let first = block_on(provider.token()).unwrap();
    assert_eq!(first.token, "gho_0");

    block_on(provider.invalidate());

    let second = block_on(provider.token()).unwrap();
    assert_eq!(second.token, "gho_1");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn token_provider_refetches_when_expired() {
    let mut client = MockIntegrationsClient::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = calls.clone();
    client.expect_get_github_token().returning(move || {
        let n = calls_clone.fetch_add(1, Ordering::SeqCst);
        // First token is already expired; second is fresh.
        let expires_at = if n == 0 {
            Utc::now() - chrono::Duration::minutes(5)
        } else {
            Utc::now() + chrono::Duration::minutes(30)
        };
        Ok(GithubTokenResponse {
            token: format!("gho_{n}"),
            expires_at: Some(expires_at.to_rfc3339()),
            installation_id: None,
        })
    });

    let provider = provider_with(client);
    assert_eq!(block_on(provider.token()).unwrap().token, "gho_0");
    // The cached token is expired, so a second call refetches.
    assert_eq!(block_on(provider.token()).unwrap().token, "gho_1");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
