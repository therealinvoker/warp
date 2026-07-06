//! API-backed per-repo GitHub model.
//!
//! Mirrors [`super::local::LocalGitHubRepoModel`] but sources PR and check
//! state from the GitHub REST API (via [`github_client::GithubClient`]) instead
//! of the `gh` CLI. It is preferred over the local (gh) path when the user has
//! a connected GitHub App installation that covers this repo (see
//! [`crate::code_review::git_repo_models::GitRepoModels::subscribe_github_repo`]).
//!
//! Enrichment beyond the gh path: [`PrInfo::checks_summary`] (aggregated from
//! check-runs + combined status) and [`PrInfo::review_comment_count`].

use std::sync::Arc;
use std::time::Duration;

use github_client::types::PrState;
use github_client::GithubClient;
use warpui::r#async::SpawnedFutureHandle;
use warpui::{Entity, ModelContext, ModelHandle};

use super::GitHubRepoEvent;
use crate::code_review::git_repo_model::{GitRepoStatusEvent, GitRepoStatusModel};
use crate::util::git::{ChecksSummary, PrInfo, RepositoryInfo};

const PR_INFO_FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const GITHUB_INFO_PERIODIC_REFRESH: Duration = Duration::from_secs(60);

/// Per-repo model that owns GitHub metadata sourced from the REST API.
pub struct ApiBackedGitHubRepoModel {
    owner: String,
    repo: String,
    /// Strong handle to the sibling git-status model (branch source).
    git_status: ModelHandle<GitRepoStatusModel>,
    branch: Option<String>,
    client: Arc<GithubClient>,
    pr_info: Option<PrInfo>,
    repository_info: Option<RepositoryInfo>,
    refreshing_pr_info_abort_handle: Option<SpawnedFutureHandle>,
    periodic_refresh_handle: Option<SpawnedFutureHandle>,
}

impl Entity for ApiBackedGitHubRepoModel {
    type Event = GitHubRepoEvent;
}

/// The data fetched for a branch's PR, assembled off the event loop.
struct FetchedPrData {
    pr_info: Option<PrInfo>,
}

impl ApiBackedGitHubRepoModel {
    pub(crate) fn new(
        owner: String,
        repo: String,
        git_status: ModelHandle<GitRepoStatusModel>,
        client: Arc<GithubClient>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let branch = git_status
            .as_ref(ctx)
            .metadata(ctx)
            .map(|m| m.current_branch_name.clone());

        ctx.subscribe_to_model(&git_status, |me, _, event, ctx| match event {
            GitRepoStatusEvent::MetadataChanged => {
                let new_branch = me
                    .git_status
                    .as_ref(ctx)
                    .metadata(ctx)
                    .map(|m| m.current_branch_name.clone());
                if new_branch != me.branch {
                    me.branch = new_branch;
                    if me.pr_info.take().is_some() {
                        ctx.emit(GitHubRepoEvent::PrInfoChanged);
                    }
                    if let Some(handle) = me.refreshing_pr_info_abort_handle.take() {
                        handle.abort();
                    }
                    me.refresh_pr_info(ctx);
                }
            }
        });

        // repository_info is known at construction (owner/repo were resolved to
        // decide on the API-backed path), so seed it eagerly.
        let repository_info = Some(RepositoryInfo {
            name: repo.clone(),
            owner: Some(owner.clone()),
        });

        let mut model = Self {
            owner,
            repo,
            git_status,
            branch,
            client,
            pr_info: None,
            repository_info,
            refreshing_pr_info_abort_handle: None,
            periodic_refresh_handle: None,
        };

        model.schedule_periodic_refresh(ctx);
        if model.branch.is_some() {
            model.refresh_pr_info(ctx);
        }
        model
    }

    fn schedule_periodic_refresh(&mut self, ctx: &mut ModelContext<Self>) {
        let handle = ctx.spawn(
            async {
                async_io::Timer::after(GITHUB_INFO_PERIODIC_REFRESH).await;
            },
            |me, _, ctx| {
                me.refresh_pr_info(ctx);
                me.schedule_periodic_refresh(ctx);
            },
        );
        self.periodic_refresh_handle = Some(handle);
    }

    pub fn pr_info(&self) -> Option<&PrInfo> {
        self.pr_info.as_ref()
    }

    pub fn repository_info(&self) -> Option<&RepositoryInfo> {
        self.repository_info.as_ref()
    }

    pub fn is_refreshing_pr_info(&self) -> bool {
        self.refreshing_pr_info_abort_handle.is_some()
    }

    /// Repository info is fixed at construction for the API path; nothing to
    /// re-fetch, so this is a no-op kept for interface parity with the local
    /// model.
    pub fn refresh_repository_info(&mut self, _ctx: &mut ModelContext<Self>) {}

    pub fn refresh_pr_info(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(branch) = self.branch.clone() else {
            return;
        };
        if self.refreshing_pr_info_abort_handle.is_some() {
            return;
        }
        let client = self.client.clone();
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let branch_for_callback = branch.clone();
        let abort_handle = ctx.spawn(
            async move {
                let fetch = fetch_pr_for_branch(&client, &owner, &repo, &branch);
                let timeout = async_io::Timer::after(PR_INFO_FETCH_TIMEOUT);
                futures::pin_mut!(fetch);
                match futures::future::select(fetch, timeout).await {
                    futures::future::Either::Left((result, _)) => result,
                    futures::future::Either::Right((_, _)) => {
                        Err(anyhow::anyhow!("PR info fetch timed out"))
                    }
                }
            },
            move |me, result, ctx| {
                me.refreshing_pr_info_abort_handle = None;
                me.handle_fetch_result(result, branch_for_callback, ctx);
            },
        );
        self.refreshing_pr_info_abort_handle = Some(abort_handle);
    }

    fn handle_fetch_result(
        &mut self,
        result: anyhow::Result<FetchedPrData>,
        branch: String,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Ok(data) => {
                if self.branch.as_deref() == Some(branch.as_str()) {
                    let changed = self.pr_info.as_ref() != data.pr_info.as_ref();
                    self.pr_info = data.pr_info;
                    if changed {
                        ctx.emit(GitHubRepoEvent::PrInfoChanged);
                    }
                }
            }
            Err(err) => {
                // Keep existing PR info on transient errors to avoid UI flashing.
                log::debug!("ApiBackedGitHubRepoModel: PR info load failed: {err:#}");
            }
        }
    }
}

/// Fetch and assemble PR info (with checks + review count) for `branch`.
async fn fetch_pr_for_branch(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    branch: &str,
) -> anyhow::Result<FetchedPrData> {
    let prs = client.list_pull_requests(owner, repo, "open").await?;
    // GitHub returns head refs as short branch names for same-repo PRs.
    let Some(pr) = prs.into_iter().find(|pr| pr.head.ref_name == branch) else {
        return Ok(FetchedPrData { pr_info: None });
    };

    let head_sha = pr.head.sha.clone();
    let checks_summary = fetch_checks_summary(client, owner, repo, &head_sha)
        .await
        .unwrap_or(None);

    let state = match pr.state {
        PrState::Open => "OPEN",
        PrState::Closed => {
            if pr.merged.unwrap_or(false) {
                "MERGED"
            } else {
                "CLOSED"
            }
        }
        PrState::Unknown => "OPEN",
    }
    .to_string();

    let pr_info = PrInfo {
        number: pr.number,
        url: pr.html_url,
        state,
        draft: pr.draft,
        base_branch: pr.base.ref_name,
        checks_summary,
        review_comment_count: pr.review_comments,
    };

    Ok(FetchedPrData {
        pr_info: Some(pr_info),
    })
}

/// Aggregate check-runs and the combined legacy status for a head SHA into a
/// [`ChecksSummary`]. Returns `Ok(None)` when there are no checks at all.
async fn fetch_checks_summary(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    head_sha: &str,
) -> anyhow::Result<Option<ChecksSummary>> {
    let mut summary = ChecksSummary::default();

    // Check runs (the modern Checks API).
    if let Ok(runs) = client.list_check_runs_for_ref(owner, repo, head_sha).await {
        for run in runs {
            if run.status != "completed" {
                summary.pending += 1;
                continue;
            }
            match run.conclusion.as_deref() {
                Some("success") | Some("neutral") | Some("skipped") => summary.success += 1,
                Some("failure") | Some("timed_out") | Some("cancelled")
                | Some("action_required") | Some("stale") => summary.failure += 1,
                _ => summary.pending += 1,
            }
        }
    }

    // Legacy combined commit statuses (some CI still reports via statuses).
    if let Ok(status) = client.combined_status_for_ref(owner, repo, head_sha).await {
        for s in status.statuses {
            match s.state.as_str() {
                "success" => summary.success += 1,
                "failure" | "error" => summary.failure += 1,
                _ => summary.pending += 1,
            }
        }
    }

    if summary.total() == 0 {
        Ok(None)
    } else {
        Ok(Some(summary))
    }
}

impl Drop for ApiBackedGitHubRepoModel {
    fn drop(&mut self) {
        if let Some(h) = self.refreshing_pr_info_abort_handle.take() {
            h.abort();
        }
        if let Some(h) = self.periodic_refresh_handle.take() {
            h.abort();
        }
    }
}
