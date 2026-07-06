//! Minimal serde-mapped subset of the GitHub REST API types used by G1.
//!
//! Only the fields the client actually reads are modelled. Unknown fields are
//! ignored by serde, so upstream additions don't break deserialization.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// State of a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrState {
    Open,
    Closed,
    /// Any state we don't explicitly model.
    #[serde(other)]
    Unknown,
}

/// A repository reference embedded in other payloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repository {
    pub id: u64,
    pub name: String,
    pub full_name: String,
    #[serde(default)]
    pub private: bool,
    pub owner: Owner,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub default_branch: Option<String>,
}

/// The owner (user or org) of a repository or resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    pub login: String,
    #[serde(default)]
    pub id: u64,
}

/// A minimal user reference (comment/review author, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub login: String,
    #[serde(default)]
    pub id: u64,
    #[serde(default, rename = "type")]
    pub user_type: Option<String>,
}

/// The head/base ref of a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrRef {
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub sha: String,
    #[serde(default)]
    pub repo: Option<Repository>,
}

/// A pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub state: PrState,
    pub title: String,
    #[serde(default)]
    pub draft: bool,
    pub html_url: String,
    pub head: PrRef,
    pub base: PrRef,
    pub user: User,
    #[serde(default)]
    pub review_comments: Option<u32>,
    #[serde(default)]
    pub merged: Option<bool>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A pull request review comment (inline, attached to a diff hunk).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewComment {
    pub id: u64,
    #[serde(default)]
    pub in_reply_to_id: Option<u64>,
    pub path: String,
    #[serde(default)]
    pub diff_hunk: String,
    #[serde(default)]
    pub line: Option<u64>,
    #[serde(default)]
    pub original_line: Option<u64>,
    /// `LEFT` or `RIGHT`.
    #[serde(default)]
    pub side: Option<String>,
    pub body: String,
    pub user: User,
    pub html_url: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A submitted review on a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    pub id: u64,
    pub user: User,
    #[serde(default)]
    pub body: Option<String>,
    /// `APPROVED`, `CHANGES_REQUESTED`, `COMMENTED`, `DISMISSED`, `PENDING`.
    pub state: String,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub submitted_at: Option<DateTime<Utc>>,
}

/// A single CI check run for a ref.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckRun {
    pub id: u64,
    pub name: String,
    /// `queued`, `in_progress`, `completed`.
    pub status: String,
    /// `success`, `failure`, `neutral`, `cancelled`, `timed_out`,
    /// `action_required`, `stale`, `skipped`. `None` while not completed.
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
}

/// The envelope returned by `GET /repos/{o}/{r}/commits/{ref}/check-runs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckRunsResponse {
    pub total_count: u64,
    pub check_runs: Vec<CheckRun>,
}

/// The combined legacy commit status for a ref
/// (`GET /repos/{o}/{r}/commits/{ref}/status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombinedStatus {
    /// `success`, `pending`, `failure`.
    pub state: String,
    pub total_count: u64,
    #[serde(default)]
    pub statuses: Vec<CommitStatus>,
}

/// A single legacy commit status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitStatus {
    /// `success`, `pending`, `failure`, `error`.
    pub state: String,
    pub context: String,
    #[serde(default)]
    pub target_url: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// An issue (note: PRs are also issues in the GitHub API, but this models the
/// issue view used by the issue endpoints).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    /// `open` or `closed`.
    pub state: String,
    #[serde(default)]
    pub body: Option<String>,
    pub user: User,
    pub html_url: String,
    #[serde(default)]
    pub comments: u32,
    /// Present when the issue is actually a pull request.
    #[serde(default)]
    pub pull_request: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A comment on an issue (or PR conversation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueComment {
    pub id: u64,
    pub body: String,
    pub user: User,
    pub html_url: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The envelope returned by `GET /installation/repositories`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallationRepositories {
    pub total_count: u64,
    pub repositories: Vec<Repository>,
}

/// Request body for creating a pull request.
///
/// Used by the deferred (proto-blocked) `CreateGitHubPr` agent action; defined
/// here so the endpoint surface is complete, but not wired to a UI in G1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatePrRequest {
    pub title: String,
    pub head: String,
    pub base: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub draft: bool,
}
