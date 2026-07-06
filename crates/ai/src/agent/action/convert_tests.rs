use warp_multi_agent_api as api;

use crate::agent::action::AIAgentActionType;

#[test]
fn read_github_pr_tool_call_converts_to_action() {
    let action: AIAgentActionType = api::message::tool_call::ReadGithubPr {
        owner: "warpdotdev".to_string(),
        repo: "warp".to_string(),
        number: 123,
    }
    .into();
    assert_eq!(
        action,
        AIAgentActionType::ReadGithubPr {
            owner: "warpdotdev".to_string(),
            repo: "warp".to_string(),
            number: 123,
        }
    );
}

#[test]
fn list_github_pr_comments_tool_call_converts_to_action() {
    let action: AIAgentActionType = api::message::tool_call::ListGithubPrComments {
        owner: "o".to_string(),
        repo: "r".to_string(),
        number: 7,
    }
    .into();
    assert_eq!(
        action,
        AIAgentActionType::ListGithubPrComments {
            owner: "o".to_string(),
            repo: "r".to_string(),
            number: 7,
        }
    );
}

#[test]
fn create_github_pr_tool_call_converts_to_action() {
    let action: AIAgentActionType = api::message::tool_call::CreateGithubPr {
        owner: "o".to_string(),
        repo: "r".to_string(),
        title: "Add feature".to_string(),
        body: "Body".to_string(),
        head: "feature".to_string(),
        base: "main".to_string(),
        draft: true,
    }
    .into();
    let AIAgentActionType::CreateGithubPr(request) = action else {
        panic!("expected CreateGithubPr action");
    };
    assert_eq!(request.owner, "o");
    assert_eq!(request.repo, "r");
    assert_eq!(request.title, "Add feature");
    assert_eq!(request.body, "Body");
    assert_eq!(request.head, "feature");
    assert_eq!(request.base, "main");
    assert!(request.draft);
}

#[test]
fn read_github_issue_tool_call_converts_to_action() {
    let action: AIAgentActionType = api::message::tool_call::ReadGithubIssue {
        owner: "o".to_string(),
        repo: "r".to_string(),
        number: 42,
    }
    .into();
    assert_eq!(
        action,
        AIAgentActionType::ReadGithubIssue {
            owner: "o".to_string(),
            repo: "r".to_string(),
            number: 42,
        }
    );
}

#[test]
fn list_github_issues_tool_call_converts_to_action() {
    let action: AIAgentActionType = api::message::tool_call::ListGithubIssues {
        owner: "o".to_string(),
        repo: "r".to_string(),
        filter: "state=closed".to_string(),
    }
    .into();
    assert_eq!(
        action,
        AIAgentActionType::ListGithubIssues {
            owner: "o".to_string(),
            repo: "r".to_string(),
            filter: "state=closed".to_string(),
        }
    );
}

#[test]
fn reply_to_pr_comment_tool_call_converts_to_action() {
    let action: AIAgentActionType = api::message::tool_call::ReplyToPrComment {
        owner: "o".to_string(),
        repo: "r".to_string(),
        comment_id: 991,
        body: "Thanks!".to_string(),
    }
    .into();
    assert_eq!(
        action,
        AIAgentActionType::ReplyToPrComment {
            owner: "o".to_string(),
            repo: "r".to_string(),
            comment_id: 991,
            body: "Thanks!".to_string(),
        }
    );
}

#[test]
fn negative_numbers_clamp_to_zero() {
    // prost decodes absent int32 as 0 and hostile payloads could carry
    // negatives; the conversion clamps rather than wrapping.
    let action: AIAgentActionType = api::message::tool_call::ReadGithubPr {
        owner: "o".to_string(),
        repo: "r".to_string(),
        number: -5,
    }
    .into();
    assert_eq!(
        action,
        AIAgentActionType::ReadGithubPr {
            owner: "o".to_string(),
            repo: "r".to_string(),
            number: 0,
        }
    );
}

#[test]
fn cancelled_results_match_action_kinds() {
    use crate::agent::action_result::AIAgentActionResultType;

    let actions = [
        AIAgentActionType::ReadGithubPr {
            owner: "o".to_string(),
            repo: "r".to_string(),
            number: 1,
        },
        AIAgentActionType::ListGithubPrComments {
            owner: "o".to_string(),
            repo: "r".to_string(),
            number: 1,
        },
        AIAgentActionType::ReadGithubIssue {
            owner: "o".to_string(),
            repo: "r".to_string(),
            number: 1,
        },
        AIAgentActionType::ListGithubIssues {
            owner: "o".to_string(),
            repo: "r".to_string(),
            filter: String::new(),
        },
        AIAgentActionType::ReplyToPrComment {
            owner: "o".to_string(),
            repo: "r".to_string(),
            comment_id: 1,
            body: "b".to_string(),
        },
    ];
    for action in actions {
        let cancelled = action.cancelled_result();
        assert!(
            cancelled.is_cancelled(),
            "expected cancelled result for {action:?}"
        );
        assert!(matches!(
            (&action, &cancelled),
            (
                AIAgentActionType::ReadGithubPr { .. },
                AIAgentActionResultType::ReadGithubPr(_)
            ) | (
                AIAgentActionType::ListGithubPrComments { .. },
                AIAgentActionResultType::ListGithubPrComments(_)
            ) | (
                AIAgentActionType::ReadGithubIssue { .. },
                AIAgentActionResultType::ReadGithubIssue(_)
            ) | (
                AIAgentActionType::ListGithubIssues { .. },
                AIAgentActionResultType::ListGithubIssues(_)
            ) | (
                AIAgentActionType::ReplyToPrComment { .. },
                AIAgentActionResultType::ReplyToPrComment(_)
            )
        ));
    }
}
