//! Unit tests for GitHub review-comment mapping.

use ai::agent::action::CommentSide;
use chrono::TimeZone;
use github_client::types::{ReviewComment, User};

use super::*;

fn sample_comment() -> ReviewComment {
    ReviewComment {
        id: 42,
        in_reply_to_id: None,
        path: "src/main.rs".to_string(),
        diff_hunk: "@@ -10,3 +12,4 @@ fn main() {\n context\n+added\n context".to_string(),
        line: Some(14),
        original_line: Some(13),
        side: Some("RIGHT".to_string()),
        body: "nit: rename".to_string(),
        user: User {
            login: "reviewer".to_string(),
            id: 1,
            user_type: Some("User".to_string()),
        },
        html_url: "https://github.com/o/r/pull/1#discussion_r42".to_string(),
        pull_request_url: Some("https://api.github.com/repos/o/r/pulls/1".to_string()),
        created_at: chrono::Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        updated_at: chrono::Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap(),
    }
}

#[test]
fn maps_line_anchored_comment() {
    let insert = review_comment_to_insert(&sample_comment());
    assert_eq!(insert.comment_id, "42");
    assert_eq!(insert.author, "reviewer");
    assert_eq!(insert.comment_body, "nit: rename");
    assert_eq!(insert.parent_comment_id, None);
    assert_eq!(
        insert.html_url.as_deref(),
        Some("https://github.com/o/r/pull/1#discussion_r42")
    );
    // RFC3339 timestamp from updated_at.
    assert!(insert
        .last_modified_timestamp
        .starts_with("2024-01-02T03:04:05"));

    let location = insert.comment_location.expect("location");
    assert_eq!(location.relative_file_path, "src/main.rs");
    let line = location.line.expect("line");
    assert_eq!(line.comment_line_range, 14..15);
    // Hunk header `@@ -10,3 +12,4 @@` → new-file start line 12.
    assert_eq!(line.diff_hunk_line_range.start, 12);
    assert_eq!(line.side, Some(CommentSide::Right));
}

#[test]
fn maps_reply_and_left_side() {
    let mut comment = sample_comment();
    comment.in_reply_to_id = Some(7);
    comment.side = Some("LEFT".to_string());
    let insert = review_comment_to_insert(&comment);
    assert_eq!(insert.parent_comment_id, Some("7".to_string()));
    let line = insert.comment_location.unwrap().line.unwrap();
    assert_eq!(line.side, Some(CommentSide::Left));
}

#[test]
fn maps_file_level_comment_without_line() {
    let mut comment = sample_comment();
    comment.line = None;
    comment.original_line = None;
    let insert = review_comment_to_insert(&comment);
    let location = insert.comment_location.expect("location");
    assert_eq!(location.relative_file_path, "src/main.rs");
    assert!(location.line.is_none());
}

#[test]
fn hunk_new_start_parses_header() {
    assert_eq!(hunk_new_start("@@ -10,3 +12,4 @@ fn main"), Some(12));
    assert_eq!(hunk_new_start("@@ -1 +1 @@"), Some(1));
    assert_eq!(hunk_new_start("not a hunk"), None);
    assert_eq!(hunk_new_start(""), None);
}
