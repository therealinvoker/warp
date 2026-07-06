//! Mapping GitHub PR review comments into Warp's imported-comment pipeline.
//!
//! Read-only in G1: comments fetched from the GitHub API are converted to
//! [`PendingImportedReviewComment`]s (via the existing
//! [`convert_insert_review_comments`] path) and rendered as
//! [`CommentOrigin::ImportedFromGitHub`]. Replying is proto-blocked and
//! deferred (see the TODO in [`fetch_pr_review_comments`]).

use ai::agent::action::{
    CommentSide, InsertReviewComment, InsertedCommentLine, InsertedCommentLocation,
};
use github_client::types::ReviewComment;
use github_client::GithubClient;

use crate::code_review::comments::{convert_insert_review_comments, PendingImportedReviewComment};

/// Parse the new-file starting line from a unified diff hunk header, e.g.
/// `@@ -12,3 +45,6 @@` → `45`. Returns `None` if the header is malformed.
fn hunk_new_start(diff_hunk: &str) -> Option<usize> {
    let header = diff_hunk.lines().next()?;
    let after_plus = header.split('+').nth(1)?;
    let start = after_plus.split([',', ' ']).next()?;
    start.trim().parse::<usize>().ok()
}

/// Map a GitHub API [`ReviewComment`] to an [`InsertReviewComment`], the input
/// type accepted by [`convert_insert_review_comments`].
///
/// `path`/`line`/`diff_hunk`/`side` map to a line-anchored location; a comment
/// without a line resolves to a file-level comment.
pub fn review_comment_to_insert(comment: &ReviewComment) -> InsertReviewComment {
    let line = comment.line.or(comment.original_line);
    let side = match comment.side.as_deref() {
        Some("LEFT") => Some(CommentSide::Left),
        Some("RIGHT") => Some(CommentSide::Right),
        _ => None,
    };

    let comment_location = Some(InsertedCommentLocation {
        relative_file_path: comment.path.clone(),
        line: line.map(|line| {
            let line = line as usize;
            // The diff hunk header gives the new-file starting line of the
            // hunk; fall back to the comment line if it can't be parsed.
            let hunk_start = hunk_new_start(&comment.diff_hunk).unwrap_or(line);
            let hunk_len = comment.diff_hunk.lines().count().saturating_sub(1).max(1);
            InsertedCommentLine {
                comment_line_range: line..line + 1,
                diff_hunk_line_range: hunk_start..hunk_start + hunk_len,
                diff_hunk_text: comment.diff_hunk.clone(),
                side,
            }
        }),
    });

    InsertReviewComment {
        comment_id: comment.id.to_string(),
        author: comment.user.login.clone(),
        last_modified_timestamp: comment.updated_at.to_rfc3339(),
        comment_body: comment.body.clone(),
        parent_comment_id: comment.in_reply_to_id.map(|id| id.to_string()),
        comment_location,
        html_url: Some(comment.html_url.clone()),
    }
}

/// Fetch the review comments for a PR and convert them into pending imported
/// comments ready to feed into the code-review comment batch.
///
/// TODO(agent-actions, proto-blocked): replying to these imported comments
/// (`ReplyToPrComment`) is deferred until the warp-proto-apis fork lands. For
/// now the overlay is strictly read-only.
pub async fn fetch_pr_review_comments(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> anyhow::Result<Vec<PendingImportedReviewComment>> {
    let comments = client
        .list_pr_review_comments(owner, repo, pr_number)
        .await?;
    let inserts: Vec<InsertReviewComment> = comments.iter().map(review_comment_to_insert).collect();
    Ok(convert_insert_review_comments(&inserts))
}

#[cfg(test)]
#[path = "pr_review_comments_tests.rs"]
mod tests;
