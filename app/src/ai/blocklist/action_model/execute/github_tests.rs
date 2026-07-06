use super::*;

#[test]
fn read_actions_are_auto_executable_and_writes_are_not() {
    let read = AIAgentActionType::ReadGithubPr {
        owner: "o".into(),
        repo: "r".into(),
        number: 1,
    };
    let list_comments = AIAgentActionType::ListGithubPrComments {
        owner: "o".into(),
        repo: "r".into(),
        number: 1,
    };
    let read_issue = AIAgentActionType::ReadGithubIssue {
        owner: "o".into(),
        repo: "r".into(),
        number: 1,
    };
    let list_issues = AIAgentActionType::ListGithubIssues {
        owner: "o".into(),
        repo: "r".into(),
        filter: String::new(),
    };
    let create_pr = AIAgentActionType::CreateGithubPr(crate::ai::agent::CreateGithubPrRequest {
        owner: "o".into(),
        repo: "r".into(),
        title: "t".into(),
        body: String::new(),
        head: "h".into(),
        base: "b".into(),
        draft: false,
    });
    let reply = AIAgentActionType::ReplyToPrComment {
        owner: "o".into(),
        repo: "r".into(),
        comment_id: 7,
        body: "b".into(),
    };

    assert!(is_github_read_action(&read));
    assert!(is_github_read_action(&list_comments));
    assert!(is_github_read_action(&read_issue));
    assert!(is_github_read_action(&list_issues));
    // Writes must never auto-execute: they go through approval gating.
    assert!(!is_github_read_action(&create_pr));
    assert!(!is_github_read_action(&reply));
}

#[test]
fn error_result_matches_action_kind() {
    let action = AIAgentActionType::ReadGithubIssue {
        owner: "o".into(),
        repo: "r".into(),
        number: 3,
    };
    match error_result_for(&action, "nope".into()) {
        AIAgentActionResultType::ReadGithubIssue(ReadGithubIssueResult::Error(message)) => {
            assert_eq!(message, "nope");
        }
        other => panic!("unexpected result: {other:?}"),
    }

    let action = AIAgentActionType::ReplyToPrComment {
        owner: "o".into(),
        repo: "r".into(),
        comment_id: 7,
        body: "b".into(),
    };
    match error_result_for(&action, "denied".into()) {
        AIAgentActionResultType::ReplyToPrComment(ReplyToPrCommentResult::Error(message)) => {
            assert_eq!(message, "denied");
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn issues_filter_parsing() {
    assert_eq!(issues_state_from_filter(""), "open");
    assert_eq!(issues_state_from_filter("open"), "open");
    assert_eq!(issues_state_from_filter("closed"), "closed");
    assert_eq!(issues_state_from_filter("all"), "all");
    assert_eq!(issues_state_from_filter("state=closed"), "closed");
    assert_eq!(issues_state_from_filter("state=all&labels=bug"), "all");
    assert_eq!(issues_state_from_filter("labels=bug&state=closed"), "closed");
    // Invalid states and junk fall back to open.
    assert_eq!(issues_state_from_filter("state=bogus"), "open");
    assert_eq!(issues_state_from_filter("what even is this"), "open");
}

#[test]
fn pull_number_parses_from_pull_request_url() {
    assert_eq!(
        pull_number_from_url("https://api.github.com/repos/o/r/pulls/1347"),
        Some(1347)
    );
    assert_eq!(
        pull_number_from_url("https://api.github.com/repos/o/r/pulls/1347/"),
        Some(1347)
    );
    assert_eq!(
        pull_number_from_url("https://api.github.com/repos/o/r/pulls/not-a-number"),
        None
    );
    assert_eq!(pull_number_from_url(""), None);
}

#[cfg(feature = "github_integration")]
mod summaries {
    use chrono::{TimeZone as _, Utc};
    use github_client::types::{CheckRun, PrRef, PrState, PullRequest, User};

    use super::super::run::{checks_summary, pr_summary_json};

    fn sample_pr() -> PullRequest {
        PullRequest {
            number: 42,
            state: PrState::Open,
            title: "Add feature".into(),
            draft: false,
            html_url: "https://github.com/o/r/pull/42".into(),
            head: PrRef {
                ref_name: "feature".into(),
                sha: "abc123".into(),
                repo: None,
            },
            base: PrRef {
                ref_name: "main".into(),
                sha: "def456".into(),
                repo: None,
            },
            user: User {
                login: "octocat".into(),
                id: 1,
                user_type: Some("User".into()),
            },
            review_comments: Some(4),
            merged: Some(false),
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).unwrap(),
        }
    }

    #[test]
    fn pr_summary_includes_key_fields_and_checks() {
        let runs = vec![
            CheckRun {
                id: 1,
                name: "build".into(),
                status: "completed".into(),
                conclusion: Some("success".into()),
                html_url: None,
            },
            CheckRun {
                id: 2,
                name: "test".into(),
                status: "completed".into(),
                conclusion: Some("failure".into()),
                html_url: None,
            },
            CheckRun {
                id: 3,
                name: "lint".into(),
                status: "in_progress".into(),
                conclusion: None,
                html_url: None,
            },
        ];
        let json: serde_json::Value =
            serde_json::from_str(&pr_summary_json(&sample_pr(), Some(&runs))).unwrap();
        assert_eq!(json["number"], 42);
        assert_eq!(json["state"], "open");
        assert_eq!(json["author"], "octocat");
        assert_eq!(json["head"]["ref"], "feature");
        assert_eq!(json["base"]["ref"], "main");
        assert_eq!(json["checks"]["total"], 3);
        assert_eq!(json["checks"]["completed"], 2);
        assert_eq!(json["checks"]["failed"], 1);
    }

    #[test]
    fn pr_summary_without_checks_omits_checks_key() {
        let json: serde_json::Value =
            serde_json::from_str(&pr_summary_json(&sample_pr(), None)).unwrap();
        assert!(json.get("checks").is_none());
    }

    #[test]
    fn checks_summary_counts_failure_variants() {
        let mk = |status: &str, conclusion: Option<&str>| CheckRun {
            id: 0,
            name: "c".into(),
            status: status.into(),
            conclusion: conclusion.map(Into::into),
            html_url: None,
        };
        let runs = vec![
            mk("completed", Some("success")),
            mk("completed", Some("timed_out")),
            mk("completed", Some("cancelled")),
            mk("completed", Some("action_required")),
            mk("queued", None),
        ];
        let summary = checks_summary(&runs);
        assert_eq!(summary["total"], 5);
        assert_eq!(summary["completed"], 4);
        assert_eq!(summary["failed"], 3);
    }
}
