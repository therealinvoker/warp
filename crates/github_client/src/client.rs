//! The GitHub REST client.

use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{Error, Result};
use crate::token::TokenProvider;
use crate::types::{
    CheckRun, CheckRunsResponse, CombinedStatus, CreatePrRequest, InstallationRepositories, Issue,
    IssueComment, PullRequest, Repository, Review, ReviewComment,
};

/// Default GitHub REST API base.
pub const DEFAULT_BASE_URL: &str = "https://api.github.com";
/// The API version pinned via the `X-GitHub-Api-Version` header.
pub const GITHUB_API_VERSION: &str = "2022-11-28";
/// Per-request timeout, matching `github_repo_model`'s gh-CLI fetch timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Maximum time we'll wait when honoring a `Retry-After` header before giving
/// up (so a hostile/broken server can't make us hang far past our timeout).
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

/// A thin, typed client over the GitHub REST API.
///
/// Auth is delegated to a [`TokenProvider`]. On a `401` the client invalidates
/// the provider's cached token and retries the request exactly once with a
/// freshly-minted token. `Retry-After` is honored once for `403`/`429`.
#[derive(Clone)]
pub struct GithubClient {
    http: reqwest::Client,
    token_provider: Arc<dyn TokenProvider>,
    base_url: String,
}

impl GithubClient {
    /// Construct a client against the default `api.github.com` base.
    pub fn new(token_provider: Arc<dyn TokenProvider>) -> Result<Self> {
        Self::with_base_url(token_provider, DEFAULT_BASE_URL.to_string())
    }

    /// Construct a client against a custom base URL (used by tests and, later,
    /// GitHub Enterprise Server).
    pub fn with_base_url(token_provider: Arc<dyn TokenProvider>, base_url: String) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent("warp-terminal")
            .build()
            .map_err(Error::Http)?;
        Ok(Self {
            http,
            token_provider,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    async fn auth_headers(&self) -> Result<HeaderMap> {
        let token = self.token_provider.token().await.map_err(Error::Token)?;
        let mut headers = HeaderMap::new();
        let mut auth = HeaderValue::from_str(&format!("Bearer {}", token.token))
            .map_err(|_| Error::InvalidToken)?;
        auth.set_sensitive(true);
        headers.insert(AUTHORIZATION, auth);
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            "X-GitHub-Api-Version",
            HeaderValue::from_static(GITHUB_API_VERSION),
        );
        headers.insert(USER_AGENT, HeaderValue::from_static("warp-terminal"));
        Ok(headers)
    }

    /// Execute a request, deserializing the JSON body on success.
    ///
    /// Retries once on `401` (after invalidating the token) and once on a
    /// rate-limit response carrying a short `Retry-After`.
    async fn request<B, R>(&self, method: Method, path: &str, body: Option<&B>) -> Result<R>
    where
        B: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let mut retried_auth = false;
        let mut retried_rate_limit = false;
        loop {
            let headers = self.auth_headers().await?;
            let mut req = self
                .http
                .request(method.clone(), self.url(path))
                .headers(headers);
            if let Some(body) = body {
                req = req.json(body);
            }
            let response = req.send().await.map_err(Error::Http)?;
            let status = response.status();

            if status == StatusCode::UNAUTHORIZED && !retried_auth {
                // Token likely stale/revoked: drop the cached one and retry
                // once with a fresh token.
                retried_auth = true;
                self.token_provider.invalidate().await;
                continue;
            }

            if (status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS)
                && !retried_rate_limit
                && let Some(delay) = retry_after(&response)
            {
                retried_rate_limit = true;
                log::debug!("github_client: honoring Retry-After of {delay:?} for {path}");
                sleep(delay.min(MAX_RETRY_AFTER)).await;
                continue;
            }

            if !status.is_success() {
                let message = response.text().await.unwrap_or_default();
                return Err(Error::Status {
                    status,
                    message: truncate(message),
                });
            }

            return response.json::<R>().await.map_err(Error::Http);
        }
    }

    async fn get<R: DeserializeOwned>(&self, path: &str) -> Result<R> {
        self.request::<(), R>(Method::GET, path, None).await
    }

    // ── Pull requests ────────────────────────────────────────────────────

    /// List pull requests for a repo. `state` is one of `open`, `closed`,
    /// `all`.
    pub async fn list_pull_requests(
        &self,
        owner: &str,
        repo: &str,
        state: &str,
    ) -> Result<Vec<PullRequest>> {
        self.get(&format!(
            "repos/{owner}/{repo}/pulls?state={state}&per_page=50"
        ))
        .await
    }

    /// Get a single pull request.
    pub async fn get_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<PullRequest> {
        self.get(&format!("repos/{owner}/{repo}/pulls/{number}"))
            .await
    }

    /// List review (inline) comments on a pull request.
    pub async fn list_pr_review_comments(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<Vec<ReviewComment>> {
        self.get(&format!(
            "repos/{owner}/{repo}/pulls/{number}/comments?per_page=100"
        ))
        .await
    }

    /// List submitted reviews on a pull request.
    pub async fn list_pr_reviews(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<Vec<Review>> {
        self.get(&format!(
            "repos/{owner}/{repo}/pulls/{number}/reviews?per_page=100"
        ))
        .await
    }

    /// Create a pull request.
    ///
    /// This is the write endpoint backing the deferred `CreateGitHubPr` agent
    /// action; it is not wired to any G1 UI.
    pub async fn create_pull_request(
        &self,
        owner: &str,
        repo: &str,
        request: &CreatePrRequest,
    ) -> Result<PullRequest> {
        self.request(
            Method::POST,
            &format!("repos/{owner}/{repo}/pulls"),
            Some(request),
        )
        .await
    }

    /// Get a single PR review comment by its id.
    ///
    /// Used to resolve the owning pull request (via `pull_request_url`) when
    /// replying to a comment identified only by id.
    pub async fn get_pr_review_comment(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
    ) -> Result<ReviewComment> {
        self.get(&format!("repos/{owner}/{repo}/pulls/comments/{comment_id}"))
            .await
    }

    /// Reply to a PR review comment, creating a threaded reply.
    ///
    /// This is the write endpoint backing the `ReplyToPrComment` agent action.
    pub async fn reply_to_pr_review_comment(
        &self,
        owner: &str,
        repo: &str,
        pull_number: u64,
        comment_id: u64,
        body: &str,
    ) -> Result<ReviewComment> {
        #[derive(Serialize)]
        struct ReplyBody<'a> {
            body: &'a str,
        }
        self.request(
            Method::POST,
            &format!("repos/{owner}/{repo}/pulls/{pull_number}/comments/{comment_id}/replies"),
            Some(&ReplyBody { body }),
        )
        .await
    }

    // ── Checks & status ──────────────────────────────────────────────────

    /// List check runs for a git ref (branch, tag, or SHA).
    pub async fn list_check_runs_for_ref(
        &self,
        owner: &str,
        repo: &str,
        git_ref: &str,
    ) -> Result<Vec<CheckRun>> {
        let response: CheckRunsResponse = self
            .get(&format!(
                "repos/{owner}/{repo}/commits/{git_ref}/check-runs?per_page=100"
            ))
            .await?;
        Ok(response.check_runs)
    }

    /// Get the combined legacy commit status for a ref.
    pub async fn combined_status_for_ref(
        &self,
        owner: &str,
        repo: &str,
        git_ref: &str,
    ) -> Result<CombinedStatus> {
        self.get(&format!("repos/{owner}/{repo}/commits/{git_ref}/status"))
            .await
    }

    // ── Issues ───────────────────────────────────────────────────────────

    /// Get a single issue.
    pub async fn get_issue(&self, owner: &str, repo: &str, number: u64) -> Result<Issue> {
        self.get(&format!("repos/{owner}/{repo}/issues/{number}"))
            .await
    }

    /// List issues for a repo. `state` is one of `open`, `closed`, `all`.
    pub async fn list_issues(&self, owner: &str, repo: &str, state: &str) -> Result<Vec<Issue>> {
        self.get(&format!(
            "repos/{owner}/{repo}/issues?state={state}&per_page=50"
        ))
        .await
    }

    /// List comments on an issue (or PR conversation).
    pub async fn list_issue_comments(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<Vec<IssueComment>> {
        self.get(&format!(
            "repos/{owner}/{repo}/issues/{number}/comments?per_page=100"
        ))
        .await
    }

    // ── Repositories ─────────────────────────────────────────────────────

    /// Get a single repository.
    pub async fn get_repository(&self, owner: &str, repo: &str) -> Result<Repository> {
        self.get(&format!("repos/{owner}/{repo}")).await
    }

    /// List repositories the authenticated installation can access.
    pub async fn installation_repositories(&self) -> Result<Vec<Repository>> {
        let response: InstallationRepositories =
            self.get("installation/repositories?per_page=100").await?;
        Ok(response.repositories)
    }
}

/// Parse a `Retry-After` header (delta-seconds form) into a `Duration`.
fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    let value = response.headers().get("retry-after")?;
    let secs: u64 = value.to_str().ok()?.trim().parse().ok()?;
    Some(Duration::from_secs(secs))
}

/// Truncate an error body so we don't blow up logs/messages with huge HTML.
fn truncate(mut s: String) -> String {
    const MAX: usize = 500;
    if s.len() > MAX {
        s.truncate(MAX);
        s.push('…');
    }
    s
}

#[cfg(not(test))]
async fn sleep(duration: Duration) {
    tokio::time::sleep(duration).await;
}

// In tests we don't actually want to wait; the retry logic is exercised
// without real delays.
#[cfg(test)]
async fn sleep(_duration: Duration) {}
