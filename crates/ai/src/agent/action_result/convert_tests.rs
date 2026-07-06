use super::*;

#[test]
fn ask_user_question_skipped_by_auto_approve_converts_to_skipped_answers() {
    let result = api::request::input::tool_call_result::Result::from(
        AskUserQuestionResult::SkippedByAutoApprove {
            question_ids: vec!["q1".to_string(), "q2".to_string()],
        },
    );

    let api::request::input::tool_call_result::Result::AskUserQuestion(result) = result else {
        panic!("expected ask_user_question result");
    };

    let Some(api::ask_user_question_result::Result::Success(success)) = result.result else {
        panic!("expected success result");
    };

    assert_eq!(success.answers.len(), 2);
    assert_eq!(success.answers[0].question_id, "q1");
    assert_eq!(success.answers[1].question_id, "q2");
    assert!(matches!(
        success.answers[0].answer,
        Some(AskUserQuestionAnswer::Skipped(()))
    ));
    assert!(matches!(
        success.answers[1].answer,
        Some(AskUserQuestionAnswer::Skipped(()))
    ));
}

#[test]
fn github_read_pr_result_converts_to_api_success_and_error() {
    let result: api::request::input::tool_call_result::Result = ReadGithubPrResult::Success {
        pr_json: r#"{"number":1}"#.to_string(),
    }
    .try_into()
    .unwrap();
    let api::request::input::tool_call_result::Result::ReadGithubPr(result) = result else {
        panic!("expected read_github_pr result");
    };
    let Some(api::read_github_pr_result::Result::Success(success)) = result.result else {
        panic!("expected success result");
    };
    assert_eq!(success.pr_json, r#"{"number":1}"#);

    let result: api::request::input::tool_call_result::Result =
        ReadGithubPrResult::Error("boom".to_string())
            .try_into()
            .unwrap();
    let api::request::input::tool_call_result::Result::ReadGithubPr(result) = result else {
        panic!("expected read_github_pr result");
    };
    assert!(matches!(
        result.result,
        Some(api::read_github_pr_result::Result::Error(
            api::read_github_pr_result::Error { message }
        )) if message == "boom"
    ));
}

#[test]
fn github_list_pr_comments_result_converts_to_api() {
    let result: api::request::input::tool_call_result::Result =
        ListGithubPrCommentsResult::Success {
            comments_json: "[]".to_string(),
        }
        .try_into()
        .unwrap();
    let api::request::input::tool_call_result::Result::ListGithubPrComments(result) = result
    else {
        panic!("expected list_github_pr_comments result");
    };
    assert!(matches!(
        result.result,
        Some(api::list_github_pr_comments_result::Result::Success(
            api::list_github_pr_comments_result::Success { comments_json }
        )) if comments_json == "[]"
    ));
}

#[test]
fn github_create_pr_result_converts_to_api() {
    let result: api::request::input::tool_call_result::Result = CreateGithubPrResult::Success {
        url: "https://github.com/o/r/pull/9".to_string(),
        number: 9,
    }
    .try_into()
    .unwrap();
    let api::request::input::tool_call_result::Result::CreateGithubPr(result) = result else {
        panic!("expected create_github_pr result");
    };
    let Some(api::create_github_pr_result::Result::Success(success)) = result.result else {
        panic!("expected success result");
    };
    assert_eq!(success.url, "https://github.com/o/r/pull/9");
    assert_eq!(success.number, 9);
}

#[test]
fn github_issue_results_convert_to_api() {
    let result: api::request::input::tool_call_result::Result = ReadGithubIssueResult::Success {
        issue_json: r#"{"number":5}"#.to_string(),
    }
    .try_into()
    .unwrap();
    assert!(matches!(
        result,
        api::request::input::tool_call_result::Result::ReadGithubIssue(_)
    ));

    let result: api::request::input::tool_call_result::Result = ListGithubIssuesResult::Success {
        issues_json: "[]".to_string(),
    }
    .try_into()
    .unwrap();
    assert!(matches!(
        result,
        api::request::input::tool_call_result::Result::ListGithubIssues(_)
    ));
}

#[test]
fn github_reply_to_pr_comment_result_converts_to_api() {
    let result: api::request::input::tool_call_result::Result = ReplyToPrCommentResult::Success {
        comment_id: 77,
        url: "https://github.com/o/r/pull/9#discussion_r77".to_string(),
    }
    .try_into()
    .unwrap();
    let api::request::input::tool_call_result::Result::ReplyToPrComment(result) = result else {
        panic!("expected reply_to_pr_comment result");
    };
    let Some(api::reply_to_pr_comment_result::Result::Success(success)) = result.result else {
        panic!("expected success result");
    };
    assert_eq!(success.comment_id, 77);
    assert_eq!(success.url, "https://github.com/o/r/pull/9#discussion_r77");
}

#[test]
fn github_cancelled_results_are_ignored_on_the_wire() {
    // Cancelled results are represented by the generic ToolCallResult.Cancel
    // marker, synthesized elsewhere; the per-result conversion must Ignore.
    for result in [
        api::request::input::tool_call_result::Result::try_from(ReadGithubPrResult::Cancelled),
        ListGithubPrCommentsResult::Cancelled.try_into(),
        CreateGithubPrResult::Cancelled.try_into(),
        ReadGithubIssueResult::Cancelled.try_into(),
        ListGithubIssuesResult::Cancelled.try_into(),
        ReplyToPrCommentResult::Cancelled.try_into(),
    ] {
        assert!(matches!(result, Err(ConvertToAPITypeError::Ignore)));
    }
}
