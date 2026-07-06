//! Unit tests for [`crate::GithubClient`] against a local mockito HTTP stub.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::Utc;

use crate::token::{GithubToken, TokenProvider};
use crate::types::PrState;
use crate::{Error, GithubClient};

/// A token provider that hands out a fixed token and counts how many times it
/// was asked / invalidated, so tests can assert the retry behavior.
struct StubTokenProvider {
    token: String,
    fetches: AtomicUsize,
    invalidations: AtomicUsize,
}

impl StubTokenProvider {
    fn new(token: &str) -> Arc<Self> {
        Arc::new(Self {
            token: token.to_string(),
            fetches: AtomicUsize::new(0),
            invalidations: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl TokenProvider for StubTokenProvider {
    async fn token(&self) -> anyhow::Result<GithubToken> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        Ok(GithubToken {
            token: self.token.clone(),
            expires_at: Some(Utc::now() + chrono::Duration::minutes(30)),
            installation_id: Some(42),
        })
    }

    async fn invalidate(&self) {
        self.invalidations.fetch_add(1, Ordering::SeqCst);
    }
}

fn client(server: &mockito::ServerGuard, provider: Arc<StubTokenProvider>) -> GithubClient {
    GithubClient::with_base_url(provider, server.url()).unwrap()
}

#[tokio::test]
async fn get_pull_request_deserializes() {
    let mut server = mockito::Server::new_async().await;
    let body = r#"{
        "number": 123,
        "state": "open",
        "title": "Add feature",
        "draft": true,
        "html_url": "https://github.com/o/r/pull/123",
        "head": {"ref": "feature", "sha": "abc123"},
        "base": {"ref": "main", "sha": "def456"},
        "user": {"login": "octocat", "id": 1, "type": "User"},
        "review_comments": 4,
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": "2024-01-02T00:00:00Z"
    }"#;
    let m = server
        .mock("GET", "/repos/o/r/pulls/123")
        .match_header("authorization", "Bearer tok")
        .match_header("x-github-api-version", "2022-11-28")
        .match_header("accept", "application/vnd.github+json")
        .with_status(200)
        .with_body(body)
        .create_async()
        .await;

    let provider = StubTokenProvider::new("tok");
    let client = client(&server, provider.clone());
    let pr = client.get_pull_request("o", "r", 123).await.unwrap();

    assert_eq!(pr.number, 123);
    assert_eq!(pr.state, PrState::Open);
    assert!(pr.draft);
    assert_eq!(pr.head.ref_name, "feature");
    assert_eq!(pr.user.login, "octocat");
    assert_eq!(pr.review_comments, Some(4));
    m.assert_async().await;
    assert_eq!(provider.fetches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn list_pull_requests_hits_expected_path() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/repos/o/r/pulls")
        .match_query(mockito::Matcher::UrlEncoded("state".into(), "open".into()))
        .with_status(200)
        .with_body("[]")
        .create_async()
        .await;

    let client = client(&server, StubTokenProvider::new("tok"));
    let prs = client.list_pull_requests("o", "r", "open").await.unwrap();
    assert!(prs.is_empty());
    m.assert_async().await;
}

#[tokio::test]
async fn list_pr_review_comments_deserializes() {
    let mut server = mockito::Server::new_async().await;
    let body = r#"[{
        "id": 999,
        "in_reply_to_id": null,
        "path": "src/main.rs",
        "diff_hunk": "@@ -1,3 +1,3 @@",
        "line": 10,
        "original_line": 10,
        "side": "RIGHT",
        "body": "nit: rename this",
        "user": {"login": "reviewer", "id": 2},
        "html_url": "https://github.com/o/r/pull/1#discussion_r999",
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": "2024-01-01T00:00:00Z"
    }]"#;
    let m = server
        .mock("GET", "/repos/o/r/pulls/1/comments")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_body(body)
        .create_async()
        .await;

    let client = client(&server, StubTokenProvider::new("tok"));
    let comments = client.list_pr_review_comments("o", "r", 1).await.unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].path, "src/main.rs");
    assert_eq!(comments[0].side.as_deref(), Some("RIGHT"));
    m.assert_async().await;
}

#[tokio::test]
async fn check_runs_unwraps_envelope() {
    let mut server = mockito::Server::new_async().await;
    let body = r#"{
        "total_count": 2,
        "check_runs": [
            {"id": 1, "name": "build", "status": "completed", "conclusion": "success"},
            {"id": 2, "name": "test", "status": "in_progress", "conclusion": null}
        ]
    }"#;
    let m = server
        .mock("GET", "/repos/o/r/commits/abc/check-runs")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_body(body)
        .create_async()
        .await;

    let client = client(&server, StubTokenProvider::new("tok"));
    let runs = client.list_check_runs_for_ref("o", "r", "abc").await.unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].conclusion.as_deref(), Some("success"));
    assert_eq!(runs[1].status, "in_progress");
    m.assert_async().await;
}

#[tokio::test]
async fn installation_repositories_unwraps_envelope() {
    let mut server = mockito::Server::new_async().await;
    let body = r#"{
        "total_count": 1,
        "repositories": [
            {"id": 5, "name": "warp", "full_name": "warpdotdev/warp",
             "private": false, "owner": {"login": "warpdotdev", "id": 9},
             "html_url": "https://github.com/warpdotdev/warp"}
        ]
    }"#;
    let m = server
        .mock("GET", "/installation/repositories")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_body(body)
        .create_async()
        .await;

    let client = client(&server, StubTokenProvider::new("tok"));
    let repos = client.installation_repositories().await.unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].full_name, "warpdotdev/warp");
    m.assert_async().await;
}

#[tokio::test]
async fn retries_once_on_401_then_succeeds() {
    let mut server = mockito::Server::new_async().await;
    // First call: 401. Second call (after token invalidation): 200.
    let unauthorized = server
        .mock("GET", "/repos/o/r")
        .with_status(401)
        .with_body(r#"{"message":"Bad credentials"}"#)
        .expect(1)
        .create_async()
        .await;
    let ok = server
        .mock("GET", "/repos/o/r")
        .with_status(200)
        .with_body(
            r#"{"id":1,"name":"r","full_name":"o/r","private":false,"owner":{"login":"o","id":1},"html_url":"https://github.com/o/r"}"#,
        )
        .expect(1)
        .create_async()
        .await;

    let provider = StubTokenProvider::new("tok");
    let client = client(&server, provider.clone());
    let repo = client.get_repository("o", "r").await.unwrap();
    assert_eq!(repo.full_name, "o/r");

    unauthorized.assert_async().await;
    ok.assert_async().await;
    // Token invalidated once, fetched twice (once per attempt).
    assert_eq!(provider.invalidations.load(Ordering::SeqCst), 1);
    assert_eq!(provider.fetches.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn does_not_retry_401_more_than_once() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/repos/o/r")
        .with_status(401)
        .with_body(r#"{"message":"Bad credentials"}"#)
        // Exactly two attempts total (initial + one retry), no more.
        .expect(2)
        .create_async()
        .await;

    let provider = StubTokenProvider::new("tok");
    let client = client(&server, provider.clone());
    let err = client.get_repository("o", "r").await.unwrap_err();
    assert_eq!(err.status(), Some(reqwest::StatusCode::UNAUTHORIZED));
    m.assert_async().await;
    assert_eq!(provider.invalidations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn honors_retry_after_on_429() {
    let mut server = mockito::Server::new_async().await;
    let limited = server
        .mock("GET", "/repos/o/r/issues/7")
        .with_status(429)
        .with_header("retry-after", "0")
        .expect(1)
        .create_async()
        .await;
    let ok = server
        .mock("GET", "/repos/o/r/issues/7")
        .with_status(200)
        .with_body(
            r#"{"number":7,"title":"bug","state":"open","user":{"login":"u","id":1},
                "html_url":"https://github.com/o/r/issues/7","comments":0,
                "created_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}"#,
        )
        .expect(1)
        .create_async()
        .await;

    let client = client(&server, StubTokenProvider::new("tok"));
    let issue = client.get_issue("o", "r", 7).await.unwrap();
    assert_eq!(issue.number, 7);
    limited.assert_async().await;
    ok.assert_async().await;
}

#[tokio::test]
async fn surfaces_non_success_status() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/repos/o/r/pulls/404")
        .with_status(404)
        .with_body(r#"{"message":"Not Found"}"#)
        .create_async()
        .await;

    let client = client(&server, StubTokenProvider::new("tok"));
    let err = client.get_pull_request("o", "r", 404).await.unwrap_err();
    assert!(err.is_not_found());
    assert!(matches!(err, Error::Status { .. }));
    m.assert_async().await;
}

#[tokio::test]
async fn get_pr_review_comment_deserializes_pull_request_url() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/repos/o/r/pulls/comments/55")
        .with_status(200)
        .with_body(
            r#"{"id":55,"path":"src/main.rs","body":"nit","user":{"login":"u","id":1},
                "html_url":"https://github.com/o/r/pull/9#discussion_r55",
                "pull_request_url":"https://api.github.com/repos/o/r/pulls/9",
                "created_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}"#,
        )
        .create_async()
        .await;

    let client = client(&server, StubTokenProvider::new("tok"));
    let comment = client.get_pr_review_comment("o", "r", 55).await.unwrap();
    assert_eq!(comment.id, 55);
    assert_eq!(
        comment.pull_request_url.as_deref(),
        Some("https://api.github.com/repos/o/r/pulls/9")
    );
    m.assert_async().await;
}

#[tokio::test]
async fn reply_to_pr_review_comment_posts_body() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/repos/o/r/pulls/9/comments/55/replies")
        .match_header("authorization", "Bearer tok")
        .match_body(mockito::Matcher::JsonString(
            r#"{"body":"Thanks!"}"#.to_string(),
        ))
        .with_status(201)
        .with_body(
            r#"{"id":77,"in_reply_to_id":55,"path":"src/main.rs","body":"Thanks!",
                "user":{"login":"me","id":2},
                "html_url":"https://github.com/o/r/pull/9#discussion_r77",
                "pull_request_url":"https://api.github.com/repos/o/r/pulls/9",
                "created_at":"2024-01-02T00:00:00Z","updated_at":"2024-01-02T00:00:00Z"}"#,
        )
        .create_async()
        .await;

    let client = client(&server, StubTokenProvider::new("tok"));
    let reply = client
        .reply_to_pr_review_comment("o", "r", 9, 55, "Thanks!")
        .await
        .unwrap();
    assert_eq!(reply.id, 77);
    assert_eq!(reply.in_reply_to_id, Some(55));
    assert_eq!(
        reply.html_url,
        "https://github.com/o/r/pull/9#discussion_r77"
    );
    m.assert_async().await;
}
