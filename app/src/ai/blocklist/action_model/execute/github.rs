//! Executor for the six GitHub agent actions (G1).
//!
//! Read actions (`ReadGithubPr`, `ListGithubPrComments`, `ReadGithubIssue`,
//! `ListGithubIssues`) auto-execute. Write actions (`CreateGithubPr`,
//! `ReplyToPrComment`) never auto-execute: they flow through the standard
//! action-approval gating (`should_autoexecute` returning `false` surfaces the
//! action-confirmation card) so the user explicitly approves every GitHub
//! write.
//!
//! All GitHub calls go through the [`github_client::GithubClient`] built from
//! the [`crate::github::GithubConnection`] singleton's token provider, and run
//! inside an `async_compat` shim because reqwest requires a Tokio reactor
//! while the app executor is not Tokio.

use futures::future::BoxFuture;
use futures::FutureExt;
#[cfg(feature = "github_integration")]
use warpui::SingletonEntity as _;
use warpui::{Entity, EntityId, ModelContext};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::{
    AIAgentActionResultType, AIAgentActionType, CreateGithubPrResult, ListGithubIssuesResult,
    ListGithubPrCommentsResult, ReadGithubIssueResult, ReadGithubPrResult, ReplyToPrCommentResult,
};
#[cfg(feature = "github_integration")]
use crate::github::GithubConnection;

pub struct GithubActionExecutor {
    #[allow(dead_code)]
    terminal_view_id: EntityId,
}

impl Entity for GithubActionExecutor {
    type Event = ();
}

impl GithubActionExecutor {
    pub fn new(terminal_view_id: EntityId) -> Self {
        Self { terminal_view_id }
    }

    /// Reads auto-execute; writes always require explicit user approval via
    /// the standard action-confirmation flow.
    pub(super) fn should_autoexecute(
        &self,
        input: ExecuteActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> bool {
        is_github_read_action(&input.action.action)
    }

    #[cfg_attr(
        not(feature = "github_integration"),
        allow(unused_variables, clippy::needless_return)
    )]
    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        // TODO(G4): governance enforcement goes here — resolve the effective
        // workspace GitHub policy (mode + repo allowlist, mirroring
        // McpGovernance) and refuse execution for disallowed repos.
        // Authoritative enforcement is server-side at token minting; this is
        // client-side defense-in-depth.
        #[cfg(not(feature = "github_integration"))]
        {
            ActionExecution::<AIAgentActionResultType>::Sync(error_result_for(
                &input.action.action,
                "GitHub integration is not available in this build.".to_string(),
            ))
        }

        #[cfg(feature = "github_integration")]
        {
            use warp_core::features::FeatureFlag;

            if !FeatureFlag::GithubIntegration.is_enabled() {
                return ActionExecution::Sync(error_result_for(
                    &input.action.action,
                    "GitHub integration is disabled.".to_string(),
                ));
            }

            let connection = GithubConnection::handle(ctx);
            if !connection.as_ref(ctx).state().connected {
                return ActionExecution::Sync(error_result_for(
                    &input.action.action,
                    "GitHub is not connected. Connect GitHub from Settings > GitHub first."
                        .to_string(),
                ));
            }

            let token_provider =
                connection.update(ctx, |connection, ctx| connection.token_provider(ctx));
            let client = match github_client::GithubClient::new(token_provider) {
                Ok(client) => client,
                Err(err) => {
                    return ActionExecution::Sync(error_result_for(
                        &input.action.action,
                        format!("Failed to construct GitHub client: {err}"),
                    ));
                }
            };

            let action = input.action.action.clone();
            ActionExecution::new_async(
                async move {
                    // reqwest requires a Tokio reactor; the app executor is
                    // not Tokio, so run the GitHub calls inside an
                    // async-compat shim.
                    use async_compat::CompatExt as _;
                    async move { run_github_action(&client, action).await }
                        .compat()
                        .await
                },
                |result, _ctx| result,
            )
        }
    }

    pub(super) fn preprocess_action(
        &mut self,
        _action: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

/// Whether `action` is one of the read-only GitHub actions.
fn is_github_read_action(action: &AIAgentActionType) -> bool {
    matches!(
        action,
        AIAgentActionType::ReadGithubPr { .. }
            | AIAgentActionType::ListGithubPrComments { .. }
            | AIAgentActionType::ReadGithubIssue { .. }
            | AIAgentActionType::ListGithubIssues { .. }
    )
}

/// Builds the appropriate `Error` result payload for a GitHub action that
/// could not be executed.
fn error_result_for(action: &AIAgentActionType, message: String) -> AIAgentActionResultType {
    match action {
        AIAgentActionType::ReadGithubPr { .. } => {
            AIAgentActionResultType::ReadGithubPr(ReadGithubPrResult::Error(message))
        }
        AIAgentActionType::ListGithubPrComments { .. } => {
            AIAgentActionResultType::ListGithubPrComments(ListGithubPrCommentsResult::Error(
                message,
            ))
        }
        AIAgentActionType::CreateGithubPr(_) => {
            AIAgentActionResultType::CreateGithubPr(CreateGithubPrResult::Error(message))
        }
        AIAgentActionType::ReadGithubIssue { .. } => {
            AIAgentActionResultType::ReadGithubIssue(ReadGithubIssueResult::Error(message))
        }
        AIAgentActionType::ListGithubIssues { .. } => {
            AIAgentActionResultType::ListGithubIssues(ListGithubIssuesResult::Error(message))
        }
        AIAgentActionType::ReplyToPrComment { .. } => {
            AIAgentActionResultType::ReplyToPrComment(ReplyToPrCommentResult::Error(message))
        }
        other => {
            debug_assert!(
                false,
                "GithubActionExecutor invoked with non-GitHub action: {other:?}"
            );
            other.cancelled_result()
        }
    }
}

/// Extracts the effective issue `state` query value from a `ListGithubIssues`
/// filter expression.
///
/// Accepts a bare state (`open` / `closed` / `all`) or GitHub list-issues
/// query-parameter syntax (`state=closed&labels=bug`). Anything else falls
/// back to `open`.
#[cfg_attr(not(feature = "github_integration"), allow(dead_code))]
fn issues_state_from_filter(filter: &str) -> &str {
    fn is_valid_state(value: &str) -> bool {
        matches!(value, "open" | "closed" | "all")
    }

    let filter = filter.trim();
    if is_valid_state(filter) {
        return filter;
    }
    filter
        .split('&')
        .find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            let value = value.trim();
            (key.trim() == "state" && is_valid_state(value)).then_some(value)
        })
        .unwrap_or("open")
}

/// Parses the PR number from a `pull_request_url`
/// (`https://api.github.com/repos/{owner}/{repo}/pulls/{number}`).
#[cfg_attr(not(feature = "github_integration"), allow(dead_code))]
fn pull_number_from_url(url: &str) -> Option<u64> {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()?
        .parse::<u64>()
        .ok()
}

#[cfg(feature = "github_integration")]
mod run {
    use github_client::types::{CheckRun, CreatePrRequest, PullRequest};
    use github_client::GithubClient;
    use serde_json::json;

    use super::*;
    use crate::ai::agent::CreateGithubPrRequest;

    /// Dispatches a single GitHub action against the client and maps the
    /// outcome into the action-result type.
    pub(super) async fn run_github_action(
        client: &GithubClient,
        action: AIAgentActionType,
    ) -> AIAgentActionResultType {
        match action {
            AIAgentActionType::ReadGithubPr {
                owner,
                repo,
                number,
            } => AIAgentActionResultType::ReadGithubPr(
                read_github_pr(client, &owner, &repo, number).await,
            ),
            AIAgentActionType::ListGithubPrComments {
                owner,
                repo,
                number,
            } => AIAgentActionResultType::ListGithubPrComments(
                list_github_pr_comments(client, &owner, &repo, number).await,
            ),
            AIAgentActionType::CreateGithubPr(request) => {
                AIAgentActionResultType::CreateGithubPr(create_github_pr(client, request).await)
            }
            AIAgentActionType::ReadGithubIssue {
                owner,
                repo,
                number,
            } => AIAgentActionResultType::ReadGithubIssue(
                read_github_issue(client, &owner, &repo, number).await,
            ),
            AIAgentActionType::ListGithubIssues {
                owner,
                repo,
                filter,
            } => AIAgentActionResultType::ListGithubIssues(
                list_github_issues(client, &owner, &repo, &filter).await,
            ),
            AIAgentActionType::ReplyToPrComment {
                owner,
                repo,
                comment_id,
                body,
            } => AIAgentActionResultType::ReplyToPrComment(
                reply_to_pr_comment(client, &owner, &repo, comment_id, &body).await,
            ),
            other => error_result_for(&other, "Not a GitHub action.".to_string()),
        }
    }

    async fn read_github_pr(
        client: &GithubClient,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> ReadGithubPrResult {
        let pr = match client.get_pull_request(owner, repo, number).await {
            Ok(pr) => pr,
            Err(err) => {
                return ReadGithubPrResult::Error(format!("Failed to read pull request: {err}"));
            }
        };
        // Best-effort checks summary for the PR head; the PR itself is still
        // useful if the checks API errors.
        let check_runs = client
            .list_check_runs_for_ref(owner, repo, &pr.head.sha)
            .await
            .ok();
        ReadGithubPrResult::Success {
            pr_json: pr_summary_json(&pr, check_runs.as_deref()),
        }
    }

    async fn list_github_pr_comments(
        client: &GithubClient,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> ListGithubPrCommentsResult {
        match client.list_pr_review_comments(owner, repo, number).await {
            Ok(comments) => match serde_json::to_string(&comments) {
                Ok(comments_json) => ListGithubPrCommentsResult::Success { comments_json },
                Err(err) => ListGithubPrCommentsResult::Error(format!(
                    "Failed to serialize PR comments: {err}"
                )),
            },
            Err(err) => {
                ListGithubPrCommentsResult::Error(format!("Failed to list PR comments: {err}"))
            }
        }
    }

    async fn create_github_pr(
        client: &GithubClient,
        request: CreateGithubPrRequest,
    ) -> CreateGithubPrResult {
        let body = CreatePrRequest {
            title: request.title,
            head: request.head,
            base: request.base,
            body: (!request.body.is_empty()).then_some(request.body),
            draft: request.draft,
        };
        match client
            .create_pull_request(&request.owner, &request.repo, &body)
            .await
        {
            Ok(pr) => CreateGithubPrResult::Success {
                url: pr.html_url,
                number: pr.number as i64,
            },
            Err(err) => {
                CreateGithubPrResult::Error(format!("Failed to create pull request: {err}"))
            }
        }
    }

    async fn read_github_issue(
        client: &GithubClient,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> ReadGithubIssueResult {
        match client.get_issue(owner, repo, number).await {
            Ok(issue) => match serde_json::to_string(&issue) {
                Ok(issue_json) => ReadGithubIssueResult::Success { issue_json },
                Err(err) => {
                    ReadGithubIssueResult::Error(format!("Failed to serialize issue: {err}"))
                }
            },
            Err(err) => ReadGithubIssueResult::Error(format!("Failed to read issue: {err}")),
        }
    }

    async fn list_github_issues(
        client: &GithubClient,
        owner: &str,
        repo: &str,
        filter: &str,
    ) -> ListGithubIssuesResult {
        let state = issues_state_from_filter(filter);
        match client.list_issues(owner, repo, state).await {
            Ok(issues) => {
                // The issues endpoint also returns PRs; keep issues only.
                let issues: Vec<_> = issues
                    .into_iter()
                    .filter(|issue| issue.pull_request.is_none())
                    .collect();
                match serde_json::to_string(&issues) {
                    Ok(issues_json) => ListGithubIssuesResult::Success { issues_json },
                    Err(err) => {
                        ListGithubIssuesResult::Error(format!("Failed to serialize issues: {err}"))
                    }
                }
            }
            Err(err) => ListGithubIssuesResult::Error(format!("Failed to list issues: {err}")),
        }
    }

    async fn reply_to_pr_comment(
        client: &GithubClient,
        owner: &str,
        repo: &str,
        comment_id: u64,
        body: &str,
    ) -> ReplyToPrCommentResult {
        // The reply endpoint requires the PR number, which the tool call does
        // not carry; resolve it from the comment being replied to.
        let parent = match client.get_pr_review_comment(owner, repo, comment_id).await {
            Ok(comment) => comment,
            Err(err) => {
                return ReplyToPrCommentResult::Error(format!(
                    "Failed to look up PR comment {comment_id}: {err}"
                ));
            }
        };
        let Some(pull_number) = parent
            .pull_request_url
            .as_deref()
            .and_then(pull_number_from_url)
        else {
            return ReplyToPrCommentResult::Error(format!(
                "Could not determine the pull request for comment {comment_id}."
            ));
        };
        match client
            .reply_to_pr_review_comment(owner, repo, pull_number, comment_id, body)
            .await
        {
            Ok(reply) => ReplyToPrCommentResult::Success {
                comment_id: reply.id as i64,
                url: reply.html_url,
            },
            Err(err) => {
                ReplyToPrCommentResult::Error(format!("Failed to reply to PR comment: {err}"))
            }
        }
    }

    /// Builds the compact JSON summary returned for `ReadGithubPr`.
    pub(super) fn pr_summary_json(pr: &PullRequest, check_runs: Option<&[CheckRun]>) -> String {
        let mut summary = json!({
            "number": pr.number,
            "title": pr.title,
            "state": pr.state,
            "draft": pr.draft,
            "merged": pr.merged,
            "author": pr.user.login,
            "head": { "ref": pr.head.ref_name, "sha": pr.head.sha },
            "base": { "ref": pr.base.ref_name },
            "html_url": pr.html_url,
            "review_comment_count": pr.review_comments,
            "created_at": pr.created_at.to_rfc3339(),
            "updated_at": pr.updated_at.to_rfc3339(),
        });
        if let Some(runs) = check_runs {
            summary["checks"] = checks_summary(runs);
        }
        summary.to_string()
    }

    /// Aggregates check runs into a small pass/fail summary.
    pub(super) fn checks_summary(runs: &[CheckRun]) -> serde_json::Value {
        let completed = runs.iter().filter(|r| r.status == "completed").count();
        let failed = runs
            .iter()
            .filter(|r| {
                matches!(
                    r.conclusion.as_deref(),
                    Some("failure" | "timed_out" | "cancelled" | "action_required")
                )
            })
            .count();
        json!({
            "total": runs.len(),
            "completed": completed,
            "failed": failed,
        })
    }
}

#[cfg(feature = "github_integration")]
use run::run_github_action;

#[cfg(test)]
#[path = "github_tests.rs"]
mod tests;
