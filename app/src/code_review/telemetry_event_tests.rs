use super::*;

/// Regression guard for the diff-chip inconsistency: the git-status code-diff
/// button always reflects uncommitted (vs-`HEAD`) changes, so opening the
/// review pane from it must reset the shared/cached `DiffStateModel` to `Head`.
/// Every other entrypoint must preserve whatever base the user last selected
/// (e.g. `master`, or a base set by importing PR comments), otherwise those
/// flows would lose the user's chosen base.
///
/// If a new `CodeReviewPaneEntrypoint` variant is added, extend the list below
/// so this stays exhaustive.
#[test]
fn only_git_diff_chip_resets_diff_to_uncommitted() {
    assert!(
        CodeReviewPaneEntrypoint::GitDiffChip.resets_diff_to_uncommitted(),
        "the git-diff chip must reopen the uncommitted diff to match its label"
    );

    for entrypoint in [
        CodeReviewPaneEntrypoint::AgentModeCompleted,
        CodeReviewPaneEntrypoint::AgentModeRunning,
        CodeReviewPaneEntrypoint::SlashCommand,
        CodeReviewPaneEntrypoint::InvokedByAgent,
        CodeReviewPaneEntrypoint::ForceOpened,
        CodeReviewPaneEntrypoint::CodeDiffHeader,
        CodeReviewPaneEntrypoint::PaneHeader,
        CodeReviewPaneEntrypoint::RightPanel,
        CodeReviewPaneEntrypoint::CLIAgentView,
        CodeReviewPaneEntrypoint::Other,
    ] {
        assert!(
            !entrypoint.resets_diff_to_uncommitted(),
            "{entrypoint} should preserve the user's selected diff base"
        );
    }
}
