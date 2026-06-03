use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use command::blocking::Command;
use warp::features::FeatureFlag;
use warp::integration_testing::code_review::{
    assert_code_review_anchor, assert_code_review_line_text, assert_code_review_loaded,
    assert_code_review_scroll_region, assert_floating_overlay_present,
    assert_general_composer_overlay_present, assert_inline_comment_block_body,
    assert_inline_comment_block_count, assert_inline_composer_body,
    assert_inline_composer_body_contains, assert_inline_composer_closed,
    assert_inline_composer_focused, assert_inline_composer_height_capped,
    assert_inline_composer_height_grew, assert_inline_composer_height_shrank,
    assert_inline_composer_height_unchanged, assert_inline_composer_open,
    assert_inline_composer_primary_label, assert_inline_composer_pushes_line_below,
    assert_inline_composer_save_disabled, assert_inline_composer_shows_remove,
    assert_line_below_y_unchanged, assert_saved_comment_count, cancel_inline_composer,
    capture_inline_composer_height, capture_line_below_baseline, cmd_enter_inline_composer,
    escape_inline_composer, open_general_composer, open_inline_composer, remove_inline_comment,
    reopen_saved_inline_comment, save_inline_composer, scroll_code_review_to_deleted_range,
    scroll_code_review_to_footer, scroll_code_review_to_header, scroll_code_review_to_line,
    set_inline_composer_body, type_into_inline_composer, ScrollRegion,
};
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::{single_terminal_view_for_tab, workspace_view};
use warp::workspace::WorkspaceAction;
use warpui_core::integration::{AssertionCallback, TestSetupUtils, TestStep};
use warpui_core::{async_assert, App, WindowId};

use super::new_builder;
use crate::util::write_all_rc_files_for_test;
use crate::Builder;

const TEST_FILE_NAME: &str = "scroll_target.txt";
const TARGET_LINE_NUMBER: usize = 70;
const INSERT_ABOVE_LINE_NUMBER: usize = 15;
const INSERT_BELOW_LINE_NUMBER: usize = 250;
const INSERTED_LINE_COUNT: usize = 10;
const TOTAL_LINE_COUNT: usize = 400;

fn base_line_text(line_number: usize) -> String {
    format!("line {line_number:03}")
}

fn modified_line_text(line_number: usize) -> String {
    format!("line {line_number:03} modified")
}

fn initial_committed_contents() -> String {
    (1..=TOTAL_LINE_COUNT)
        .map(|line_number| format!("{}\n", base_line_text(line_number)))
        .collect()
}

fn initial_diff_contents() -> String {
    (1..=TOTAL_LINE_COUNT)
        .map(|line_number| {
            let line_text =
                if (10..=80).contains(&line_number) || (200..=300).contains(&line_number) {
                    modified_line_text(line_number)
                } else {
                    base_line_text(line_number)
                };
            format!("{line_text}\n")
        })
        .collect()
}

fn inserted_lines(prefix: &str) -> Vec<String> {
    (1..=INSERTED_LINE_COUNT)
        .map(|index| format!("{prefix} inserted {index:02}"))
        .collect()
}

fn insert_lines(path: &Path, before_line_number: usize, new_lines: &[String]) {
    let contents = fs::read_to_string(path).expect("should read test file");
    let mut lines: Vec<String> = contents.lines().map(ToOwned::to_owned).collect();
    let insert_index = before_line_number.saturating_sub(1);
    lines.splice(insert_index..insert_index, new_lines.iter().cloned());
    fs::write(path, format!("{}\n", lines.join("\n"))).expect("should rewrite test file");
}

fn run_git(test_dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(test_dir)
        .status()
        .expect("git command should run");
    assert!(status.success(), "git {:?} should succeed", args);
}

fn open_code_review_panel(app: &mut App, window_id: WindowId) {
    let workspace = workspace_view(app, window_id);
    app.update(|ctx| {
        ctx.dispatch_typed_action_for_view(
            window_id,
            workspace.id(),
            &WorkspaceAction::ToggleRightPanel,
        );
    });
}

fn assert_repo_detected() -> AssertionCallback {
    Box::new(|app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
        terminal_view.read(app, |terminal_view, _ctx| {
            async_assert!(
                terminal_view.current_repo_path().is_some(),
                "expected the active terminal to detect a git repository"
            )
        })
    })
}

fn scroll_code_review_to_target_line() -> TestStep {
    scroll_code_review_to_line(TEST_FILE_NAME, TARGET_LINE_NUMBER)
        .set_timeout(Duration::from_secs(10))
        .set_retries(2)
        .add_assertion(assert_code_review_anchor(
            TEST_FILE_NAME,
            modified_line_text(TARGET_LINE_NUMBER),
            Some(TARGET_LINE_NUMBER),
        ))
        // Allow the scroll debounce (150ms) to fire so that the stored
        // scroll context is captured before the next step mutates the file.
        .set_post_step_pause(Duration::from_millis(250))
}

fn mutate_test_file(before_line_number: usize, prefix: &'static str) -> TestStep {
    TestStep::new(&format!(
        "Insert {INSERTED_LINE_COUNT} lines at {before_line_number}"
    ))
    .with_action(move |app, window_id, _| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
        let cwd = terminal_view
            .read(app, |terminal_view, _ctx| terminal_view.pwd())
            .expect("terminal should expose current working directory");
        let file_path = PathBuf::from(cwd).join(TEST_FILE_NAME);
        let new_lines = inserted_lines(prefix);
        insert_lines(&file_path, before_line_number, &new_lines);
    })
    .set_post_step_pause(Duration::from_millis(250))
}

fn code_review_scroll_anchor_builder(
    insertion_line_number: usize,
    insertion_prefix: &'static str,
) -> Builder {
    FeatureFlag::CodeReviewScrollPreservation.set_enabled(true);
    FeatureFlag::IncrementalAutoReload.set_enabled(true);
    let inserted_line_text = inserted_lines(insertion_prefix)
        .into_iter()
        .next()
        .expect("inserted lines should not be empty");
    new_builder()
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let repo_dir = test_dir.join("repo");
            fs::create_dir_all(&repo_dir).expect("should create repo subdirectory");
            let repo_dir_string = repo_dir
                .to_str()
                .expect("repo directory should be valid utf-8");

            write_all_rc_files_for_test(&test_dir, format!("cd {repo_dir_string}"));

            fs::write(repo_dir.join(TEST_FILE_NAME), initial_committed_contents())
                .expect("should write initial committed contents");
            run_git(&repo_dir, &["init", "-b", "main"]);
            run_git(&repo_dir, &["config", "user.email", "test@example.com"]);
            run_git(&repo_dir, &["config", "user.name", "Warp Integration Test"]);
            run_git(&repo_dir, &["add", TEST_FILE_NAME]);
            run_git(&repo_dir, &["commit", "-m", "Initial commit"]);

            fs::write(repo_dir.join(TEST_FILE_NAME), initial_diff_contents())
                .expect("should write initial diff contents");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Wait for the terminal to detect the git repository")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_repo_detected()),
        )
        .with_step(
            TestStep::new("Open the code review panel")
                .with_action(|app, window_id, _| open_code_review_panel(app, window_id)),
        )
        .with_step(
            TestStep::new("Wait for the code review panel to load file diffs")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_loaded()),
        )
        .with_step(scroll_code_review_to_target_line())
        .with_step(mutate_test_file(insertion_line_number, insertion_prefix))
        .with_step(
            TestStep::new("Wait for code review to reflect the inserted lines")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_line_text(
                    TEST_FILE_NAME,
                    insertion_line_number,
                    inserted_line_text,
                )),
        )
        .with_step(
            TestStep::new("Wait for code review to preserve the visible anchor text")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_anchor(
                    TEST_FILE_NAME,
                    modified_line_text(TARGET_LINE_NUMBER),
                    None,
                )),
        )
}

pub fn test_code_review_scroll_anchor_preserved_when_inserting_above() -> Builder {
    code_review_scroll_anchor_builder(INSERT_ABOVE_LINE_NUMBER, "above")
}

pub fn test_code_review_scroll_anchor_unchanged_when_inserting_below() -> Builder {
    code_review_scroll_anchor_builder(INSERT_BELOW_LINE_NUMBER, "below")
}

// --- Multi-file test ---
// Tests that scroll preservation works when scrolled to the second file in the
// code review list. This exercises the adjustment callback returning an
// item-relative offset (not absolute), which only matters for index > 0.

const SECOND_FILE_NAME: &str = "second_file.txt";
const FIRST_FILE_NAME: &str = "first_file.txt";
const MULTI_FILE_TARGET_LINE: usize = 70;
const MULTI_FILE_INSERT_LINE: usize = 15;

fn multi_file_base_line(file_prefix: &str, line_number: usize) -> String {
    format!("{file_prefix} line {line_number:03}")
}

fn multi_file_modified_line(file_prefix: &str, line_number: usize) -> String {
    format!("{file_prefix} line {line_number:03} modified")
}

fn multi_file_committed_contents(file_prefix: &str) -> String {
    (1..=TOTAL_LINE_COUNT)
        .map(|n| format!("{}\n", multi_file_base_line(file_prefix, n)))
        .collect()
}

fn multi_file_diff_contents(file_prefix: &str) -> String {
    (1..=TOTAL_LINE_COUNT)
        .map(|n| {
            let text = if (10..=80).contains(&n) || (200..=300).contains(&n) {
                multi_file_modified_line(file_prefix, n)
            } else {
                multi_file_base_line(file_prefix, n)
            };
            format!("{text}\n")
        })
        .collect()
}

fn mutate_named_file(
    file_name: &'static str,
    before_line_number: usize,
    prefix: &'static str,
) -> TestStep {
    TestStep::new(&format!(
        "Insert {INSERTED_LINE_COUNT} lines at {before_line_number} in {file_name}"
    ))
    .with_action(move |app, window_id, _| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
        let cwd = terminal_view
            .read(app, |terminal_view, _ctx| terminal_view.pwd())
            .expect("terminal should expose current working directory");
        let file_path = PathBuf::from(cwd).join(file_name);
        let new_lines = inserted_lines(prefix);
        insert_lines(&file_path, before_line_number, &new_lines);
    })
    .set_post_step_pause(Duration::from_millis(250))
}

// --- Deleted range test ---
// Tests that scroll preservation works when scrolled to a deleted (removed) line
// region. This exercises the RemovedLine variant of RelocatableScrollContext.

const DELETED_RANGE_START: usize = 61;
const DELETED_RANGE_END: usize = 80;
/// Current buffer line just before the deleted range. The temporary blocks
/// for deleted lines 61-80 appear immediately after this line in the diff.
const DELETED_RANGE_NEAR_LINE: usize = 60;

fn deleted_range_diff_contents() -> String {
    (1..=TOTAL_LINE_COUNT)
        .filter(|&n| !(DELETED_RANGE_START..=DELETED_RANGE_END).contains(&n))
        .map(|n| {
            let text = if (200..=300).contains(&n) {
                modified_line_text(n)
            } else {
                base_line_text(n)
            };
            format!("{text}\n")
        })
        .collect()
}

pub fn test_code_review_scroll_preserved_deleted_range() -> Builder {
    FeatureFlag::CodeReviewScrollPreservation.set_enabled(true);
    FeatureFlag::IncrementalAutoReload.set_enabled(true);

    let inserted_line_text = inserted_lines("above")
        .into_iter()
        .next()
        .expect("inserted lines should not be empty");

    new_builder()
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let repo_dir = test_dir.join("repo");
            fs::create_dir_all(&repo_dir).expect("should create repo subdirectory");
            let repo_dir_string = repo_dir
                .to_str()
                .expect("repo directory should be valid utf-8");

            write_all_rc_files_for_test(&test_dir, format!("cd {repo_dir_string}"));

            fs::write(repo_dir.join(TEST_FILE_NAME), initial_committed_contents())
                .expect("should write initial committed contents");
            run_git(&repo_dir, &["init", "-b", "main"]);
            run_git(&repo_dir, &["config", "user.email", "test@example.com"]);
            run_git(&repo_dir, &["config", "user.name", "Warp Integration Test"]);
            run_git(&repo_dir, &["add", TEST_FILE_NAME]);
            run_git(&repo_dir, &["commit", "-m", "Initial commit"]);

            // Write diff contents that DELETE lines 61-80
            fs::write(repo_dir.join(TEST_FILE_NAME), deleted_range_diff_contents())
                .expect("should write deleted range diff contents");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Wait for the terminal to detect the git repository")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_repo_detected()),
        )
        .with_step(
            TestStep::new("Open the code review panel")
                .with_action(|app, window_id, _| open_code_review_panel(app, window_id)),
        )
        .with_step(
            TestStep::new("Wait for the code review panel to load file diffs")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_loaded()),
        )
        .with_step(
            scroll_code_review_to_deleted_range(TEST_FILE_NAME, DELETED_RANGE_NEAR_LINE)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_code_review_scroll_region(ScrollRegion::RemovedLine))
                .set_post_step_pause(Duration::from_millis(250)),
        )
        .with_step(mutate_test_file(INSERT_ABOVE_LINE_NUMBER, "above"))
        .with_step(
            TestStep::new("Wait for code review to reflect the inserted lines")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_line_text(
                    TEST_FILE_NAME,
                    INSERT_ABOVE_LINE_NUMBER,
                    inserted_line_text,
                ))
                // Allow time for the asynchronous diff recomputation to complete.
                // Without this, the assertion below may pass against the stale
                // (pre-recompute) layout where temporary blocks haven't moved.
                .set_post_step_pause(Duration::from_millis(1000)),
        )
        .with_step(
            TestStep::new("Assert scroll is still in the deleted range after preservation")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_scroll_region(ScrollRegion::RemovedLine)),
        )
}

// --- Header range test ---
// Tests that scroll preservation works when scrolled to the file header region.
// This exercises the Header variant of RelocatableScrollContext.

pub fn test_code_review_scroll_preserved_header_range() -> Builder {
    FeatureFlag::CodeReviewScrollPreservation.set_enabled(true);
    FeatureFlag::IncrementalAutoReload.set_enabled(true);

    let inserted_line_text = inserted_lines("above")
        .into_iter()
        .next()
        .expect("inserted lines should not be empty");

    new_builder()
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let repo_dir = test_dir.join("repo");
            fs::create_dir_all(&repo_dir).expect("should create repo subdirectory");
            let repo_dir_string = repo_dir
                .to_str()
                .expect("repo directory should be valid utf-8");

            write_all_rc_files_for_test(&test_dir, format!("cd {repo_dir_string}"));

            fs::write(repo_dir.join(TEST_FILE_NAME), initial_committed_contents())
                .expect("should write initial committed contents");
            run_git(&repo_dir, &["init", "-b", "main"]);
            run_git(&repo_dir, &["config", "user.email", "test@example.com"]);
            run_git(&repo_dir, &["config", "user.name", "Warp Integration Test"]);
            run_git(&repo_dir, &["add", TEST_FILE_NAME]);
            run_git(&repo_dir, &["commit", "-m", "Initial commit"]);

            fs::write(repo_dir.join(TEST_FILE_NAME), initial_diff_contents())
                .expect("should write initial diff contents");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Wait for the terminal to detect the git repository")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_repo_detected()),
        )
        .with_step(
            TestStep::new("Open the code review panel")
                .with_action(|app, window_id, _| open_code_review_panel(app, window_id)),
        )
        .with_step(
            TestStep::new("Wait for the code review panel to load file diffs")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_loaded()),
        )
        .with_step(
            scroll_code_review_to_header(TEST_FILE_NAME)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_code_review_scroll_region(ScrollRegion::Header))
                .set_post_step_pause(Duration::from_millis(250)),
        )
        .with_step(mutate_test_file(INSERT_ABOVE_LINE_NUMBER, "above"))
        .with_step(
            TestStep::new("Wait for code review to reflect the inserted lines")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_line_text(
                    TEST_FILE_NAME,
                    INSERT_ABOVE_LINE_NUMBER,
                    inserted_line_text,
                )),
        )
        .with_step(
            TestStep::new("Assert scroll is still in the header region after preservation")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_scroll_region(ScrollRegion::Header)),
        )
}

// --- Footer range test ---
// Tests that scroll preservation works when scrolled past the editor content
// into the footer region. This exercises the Footer variant of RelocatableScrollContext.
//
// The footer region of a file is only reachable when there is a sufficiently
// tall file below it in the list; otherwise the list's max-scroll clamping
// prevents scrolling past the editor content. This test reuses the multi-file
// helpers (FIRST_FILE_NAME / SECOND_FILE_NAME) so that first_file.txt (index 0)
// has second_file.txt below it, making the footer reachable.

pub fn test_code_review_scroll_preserved_footer_range() -> Builder {
    FeatureFlag::CodeReviewScrollPreservation.set_enabled(true);
    FeatureFlag::IncrementalAutoReload.set_enabled(true);

    let inserted_line_text = inserted_lines("first")
        .into_iter()
        .next()
        .expect("inserted lines should not be empty");

    new_builder()
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let repo_dir = test_dir.join("repo");
            fs::create_dir_all(&repo_dir).expect("should create repo subdirectory");
            let repo_dir_string = repo_dir
                .to_str()
                .expect("repo directory should be valid utf-8");

            write_all_rc_files_for_test(&test_dir, format!("cd {repo_dir_string}"));

            // Two files: first_file.txt at index 0, second_file.txt at index 1.
            // We scroll to the footer of the first file; the second file
            // provides enough total list height so the footer is reachable.
            fs::write(
                repo_dir.join(FIRST_FILE_NAME),
                multi_file_committed_contents("first"),
            )
            .expect("should write first file committed contents");
            fs::write(
                repo_dir.join(SECOND_FILE_NAME),
                multi_file_committed_contents("second"),
            )
            .expect("should write second file committed contents");

            run_git(&repo_dir, &["init", "-b", "main"]);
            run_git(&repo_dir, &["config", "user.email", "test@example.com"]);
            run_git(&repo_dir, &["config", "user.name", "Warp Integration Test"]);
            run_git(&repo_dir, &["add", FIRST_FILE_NAME, SECOND_FILE_NAME]);
            run_git(&repo_dir, &["commit", "-m", "Initial commit"]);

            fs::write(
                repo_dir.join(FIRST_FILE_NAME),
                multi_file_diff_contents("first"),
            )
            .expect("should write first file diff contents");
            fs::write(
                repo_dir.join(SECOND_FILE_NAME),
                multi_file_diff_contents("second"),
            )
            .expect("should write second file diff contents");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Wait for the terminal to detect the git repository")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_repo_detected()),
        )
        .with_step(
            TestStep::new("Open the code review panel")
                .with_action(|app, window_id, _| open_code_review_panel(app, window_id)),
        )
        .with_step(
            TestStep::new("Wait for the code review panel to load file diffs")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_loaded()),
        )
        .with_step(
            scroll_code_review_to_footer(FIRST_FILE_NAME)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_code_review_scroll_region(ScrollRegion::Footer))
                .set_post_step_pause(Duration::from_millis(250)),
        )
        .with_step(mutate_named_file(
            FIRST_FILE_NAME,
            MULTI_FILE_INSERT_LINE,
            "first",
        ))
        .with_step(
            TestStep::new("Wait for code review to reflect the inserted lines")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_line_text(
                    FIRST_FILE_NAME,
                    MULTI_FILE_INSERT_LINE,
                    inserted_line_text,
                )),
        )
        .with_step(
            TestStep::new("Assert scroll is still in the footer region after preservation")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_scroll_region(ScrollRegion::Footer)),
        )
}

pub fn test_code_review_scroll_preserved_second_file() -> Builder {
    FeatureFlag::CodeReviewScrollPreservation.set_enabled(true);
    FeatureFlag::IncrementalAutoReload.set_enabled(true);

    let inserted_line_text = inserted_lines("second")
        .into_iter()
        .next()
        .expect("inserted lines should not be empty");

    new_builder()
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let repo_dir = test_dir.join("repo");
            fs::create_dir_all(&repo_dir).expect("should create repo subdirectory");
            let repo_dir_string = repo_dir
                .to_str()
                .expect("repo directory should be valid utf-8");

            write_all_rc_files_for_test(&test_dir, format!("cd {repo_dir_string}"));

            // Create and commit two files. File names sort alphabetically so
            // first_file.txt appears at index 0 and second_file.txt at index 1.
            fs::write(
                repo_dir.join(FIRST_FILE_NAME),
                multi_file_committed_contents("first"),
            )
            .expect("should write first file committed contents");
            fs::write(
                repo_dir.join(SECOND_FILE_NAME),
                multi_file_committed_contents("second"),
            )
            .expect("should write second file committed contents");

            run_git(&repo_dir, &["init", "-b", "main"]);
            run_git(&repo_dir, &["config", "user.email", "test@example.com"]);
            run_git(&repo_dir, &["config", "user.name", "Warp Integration Test"]);
            run_git(&repo_dir, &["add", FIRST_FILE_NAME, SECOND_FILE_NAME]);
            run_git(&repo_dir, &["commit", "-m", "Initial commit"]);

            // Write modified versions to create diffs in both files
            fs::write(
                repo_dir.join(FIRST_FILE_NAME),
                multi_file_diff_contents("first"),
            )
            .expect("should write first file diff contents");
            fs::write(
                repo_dir.join(SECOND_FILE_NAME),
                multi_file_diff_contents("second"),
            )
            .expect("should write second file diff contents");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Wait for the terminal to detect the git repository")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_repo_detected()),
        )
        .with_step(
            TestStep::new("Open the code review panel")
                .with_action(|app, window_id, _| open_code_review_panel(app, window_id)),
        )
        .with_step(
            TestStep::new("Wait for the code review panel to load file diffs")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_loaded()),
        )
        // Scroll to a target line in the SECOND file (index 1)
        .with_step(
            scroll_code_review_to_line(SECOND_FILE_NAME, MULTI_FILE_TARGET_LINE)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_code_review_anchor(
                    SECOND_FILE_NAME,
                    multi_file_modified_line("second", MULTI_FILE_TARGET_LINE),
                    Some(MULTI_FILE_TARGET_LINE),
                ))
                .set_post_step_pause(Duration::from_millis(250)),
        )
        // Insert lines above the target in the second file
        .with_step(mutate_named_file(
            SECOND_FILE_NAME,
            MULTI_FILE_INSERT_LINE,
            "second",
        ))
        .with_step(
            TestStep::new("Wait for code review to reflect the inserted lines in second file")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_line_text(
                    SECOND_FILE_NAME,
                    MULTI_FILE_INSERT_LINE,
                    inserted_line_text,
                )),
        )
        .with_step(
            TestStep::new("Wait for code review to preserve the visible anchor in second file")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_anchor(
                    SECOND_FILE_NAME,
                    multi_file_modified_line("second", MULTI_FILE_TARGET_LINE),
                    None,
                )),
        )
}

// =====================================================================================
// Inline comment composer tests (FeatureFlag::EmbeddedCodeReviewComments)
// =====================================================================================
//
// These boot a hermetic code review on a single modified file and exercise the inline composer
// (open/type/save/cancel/escape/reopen/remove) end-to-end, covering VAL-COMPOSER-001..014,016,017.
// The composer is opened on a line within the modified region (10-80) so it is a current, on-screen
// line in the diff editor.

/// Line we open the inline composer on (within the modified region, near the top of the editor).
const COMPOSER_LINE: usize = 20;

fn setup_single_file_repo(utils: &mut TestSetupUtils) {
    let test_dir = utils.test_dir();
    let repo_dir = test_dir.join("repo");
    fs::create_dir_all(&repo_dir).expect("should create repo subdirectory");
    let repo_dir_string = repo_dir
        .to_str()
        .expect("repo directory should be valid utf-8");

    write_all_rc_files_for_test(&test_dir, format!("cd {repo_dir_string}"));

    fs::write(repo_dir.join(TEST_FILE_NAME), initial_committed_contents())
        .expect("should write initial committed contents");
    run_git(&repo_dir, &["init", "-b", "main"]);
    run_git(&repo_dir, &["config", "user.email", "test@example.com"]);
    run_git(&repo_dir, &["config", "user.name", "Warp Integration Test"]);
    run_git(&repo_dir, &["add", TEST_FILE_NAME]);
    run_git(&repo_dir, &["commit", "-m", "Initial commit"]);

    fs::write(repo_dir.join(TEST_FILE_NAME), initial_diff_contents())
        .expect("should write initial diff contents");
}

/// A loaded single-file code review with the inline-comments flag set to `flag_enabled`.
fn composer_builder(flag_enabled: bool) -> Builder {
    FeatureFlag::EmbeddedCodeReviewComments.set_enabled(flag_enabled);

    new_builder()
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(setup_single_file_repo)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Wait for the terminal to detect the git repository")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_repo_detected()),
        )
        .with_step(
            TestStep::new("Open the code review panel")
                .with_action(|app, window_id, _| open_code_review_panel(app, window_id)),
        )
        .with_step(
            TestStep::new("Wait for the code review panel to load file diffs")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_loaded()),
        )
}

/// VAL-COMPOSER-001/002/012: opening the composer pushes the line below down, anchors at the
/// clicked line, and opening a second composer leaves exactly one (anchored at the new line).
pub fn test_code_review_composer_opens_inline_and_pushes_line() -> Builder {
    composer_builder(true)
        .with_step(
            open_inline_composer(TEST_FILE_NAME, COMPOSER_LINE)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_inline_composer_open(
                    TEST_FILE_NAME,
                    Some(COMPOSER_LINE),
                ))
                .add_assertion(assert_inline_comment_block_count(TEST_FILE_NAME, 1))
                .add_assertion(assert_inline_composer_pushes_line_below(
                    TEST_FILE_NAME,
                    COMPOSER_LINE,
                )),
        )
        .with_step(
            open_inline_composer(TEST_FILE_NAME, COMPOSER_LINE + 5)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_inline_composer_open(
                    TEST_FILE_NAME,
                    Some(COMPOSER_LINE + 5),
                ))
                .add_assertion(assert_inline_comment_block_count(TEST_FILE_NAME, 1)),
        )
}

/// VAL-COMPOSER-004/014: the primary button is disabled while empty, enabled after typing; saving
/// closes the composer, restores the layout, and persists the comment. Empty/whitespace can't save.
pub fn test_code_review_composer_save_via_button() -> Builder {
    composer_builder(true)
        .with_step(
            open_inline_composer(TEST_FILE_NAME, COMPOSER_LINE)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_inline_composer_open(
                    TEST_FILE_NAME,
                    Some(COMPOSER_LINE),
                ))
                .add_assertion(assert_inline_composer_save_disabled(TEST_FILE_NAME, true)),
        )
        // Whitespace-only drafts cannot enable the button.
        .with_step(
            type_into_inline_composer(TEST_FILE_NAME, "   ")
                .add_assertion(assert_inline_composer_save_disabled(TEST_FILE_NAME, true)),
        )
        .with_step(
            type_into_inline_composer(TEST_FILE_NAME, "looks good")
                .add_assertion(assert_inline_composer_body(TEST_FILE_NAME, "   looks good"))
                .add_assertion(assert_inline_composer_save_disabled(TEST_FILE_NAME, false)),
        )
        .with_step(
            save_inline_composer(TEST_FILE_NAME)
                .set_timeout(Duration::from_secs(10))
                .add_assertion(assert_inline_composer_closed(TEST_FILE_NAME))
                .add_assertion(assert_inline_comment_block_count(TEST_FILE_NAME, 0))
                .add_assertion(assert_saved_comment_count(1)),
        )
}

/// VAL-COMPOSER-013: typed input routes into the focused composer and does not edit the code line.
/// On open, the composer's inner editor takes focus (so keystrokes route to it, not the read-only
/// diff); the draft captures typed input while the underlying code line stays unchanged.
pub fn test_code_review_composer_typing_routes_to_composer() -> Builder {
    composer_builder(true)
        .with_step(
            open_inline_composer(TEST_FILE_NAME, COMPOSER_LINE)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .set_post_step_pause(Duration::from_millis(250))
                .add_assertion(assert_inline_composer_open(
                    TEST_FILE_NAME,
                    Some(COMPOSER_LINE),
                ))
                // The composer's inner editor holds focus, so typed input routes to it (not code).
                .add_assertion(assert_inline_composer_focused(TEST_FILE_NAME, true)),
        )
        // Real typed input dispatched at the window must not mutate the read-only diff line.
        .with_step(
            TestStep::new("Typed input does not edit the read-only diff")
                .set_timeout(Duration::from_secs(10))
                .with_typed_characters(&["h", "e", "l", "l", "o"])
                .add_assertion(assert_code_review_line_text(
                    TEST_FILE_NAME,
                    COMPOSER_LINE,
                    modified_line_text(COMPOSER_LINE),
                )),
        )
        // The draft captures input and remains the only place the text lands.
        .with_step(
            type_into_inline_composer(TEST_FILE_NAME, "routed to composer")
                .set_timeout(Duration::from_secs(10))
                .add_assertion(assert_inline_composer_body_contains(
                    TEST_FILE_NAME,
                    "routed to composer",
                ))
                .add_assertion(assert_code_review_line_text(
                    TEST_FILE_NAME,
                    COMPOSER_LINE,
                    modified_line_text(COMPOSER_LINE),
                )),
        )
}

/// VAL-COMPOSER-005: Cmd/Ctrl+Enter saves the composer (closes + persists), like the button.
pub fn test_code_review_composer_cmd_enter_saves() -> Builder {
    composer_builder(true)
        .with_step(
            open_inline_composer(TEST_FILE_NAME, COMPOSER_LINE)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_inline_composer_open(
                    TEST_FILE_NAME,
                    Some(COMPOSER_LINE),
                )),
        )
        .with_step(
            type_into_inline_composer(TEST_FILE_NAME, "saved by keyboard").add_assertion(
                assert_inline_composer_body(TEST_FILE_NAME, "saved by keyboard"),
            ),
        )
        .with_step(
            cmd_enter_inline_composer(TEST_FILE_NAME)
                .set_timeout(Duration::from_secs(10))
                .add_assertion(assert_inline_composer_closed(TEST_FILE_NAME))
                .add_assertion(assert_inline_comment_block_count(TEST_FILE_NAME, 0))
                .add_assertion(assert_saved_comment_count(1)),
        )
}

/// VAL-COMPOSER-006/007/008: Cancel closes and restores; Escape on an empty draft closes; Escape on
/// a non-empty draft keeps the composer open.
pub fn test_code_review_composer_cancel_and_escape() -> Builder {
    composer_builder(true)
        // Cancel closes + restores layout.
        .with_step(
            open_inline_composer(TEST_FILE_NAME, COMPOSER_LINE)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_inline_composer_open(
                    TEST_FILE_NAME,
                    Some(COMPOSER_LINE),
                ))
                .add_assertion(assert_inline_comment_block_count(TEST_FILE_NAME, 1)),
        )
        .with_step(
            cancel_inline_composer(TEST_FILE_NAME)
                .set_timeout(Duration::from_secs(10))
                .add_assertion(assert_inline_composer_closed(TEST_FILE_NAME))
                .add_assertion(assert_inline_comment_block_count(TEST_FILE_NAME, 0))
                .add_assertion(assert_saved_comment_count(0)),
        )
        // Escape on an empty draft closes.
        .with_step(
            open_inline_composer(TEST_FILE_NAME, COMPOSER_LINE)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_inline_composer_open(
                    TEST_FILE_NAME,
                    Some(COMPOSER_LINE),
                )),
        )
        .with_step(
            escape_inline_composer(TEST_FILE_NAME)
                .set_timeout(Duration::from_secs(10))
                .add_assertion(assert_inline_composer_closed(TEST_FILE_NAME))
                .add_assertion(assert_inline_comment_block_count(TEST_FILE_NAME, 0)),
        )
        // Escape on a non-empty draft does NOT close.
        .with_step(
            open_inline_composer(TEST_FILE_NAME, COMPOSER_LINE)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_inline_composer_open(
                    TEST_FILE_NAME,
                    Some(COMPOSER_LINE),
                )),
        )
        .with_step(
            type_into_inline_composer(TEST_FILE_NAME, "keep me")
                .add_assertion(assert_inline_composer_body(TEST_FILE_NAME, "keep me")),
        )
        .with_step(
            escape_inline_composer(TEST_FILE_NAME)
                .set_timeout(Duration::from_secs(10))
                .add_assertion(assert_inline_composer_open(
                    TEST_FILE_NAME,
                    Some(COMPOSER_LINE),
                ))
                .add_assertion(assert_inline_comment_block_count(TEST_FILE_NAME, 1)),
        )
}

/// VAL-COMPOSER-009/010: reopening a saved comment shows the prefilled inline editor with
/// "Update"/Remove; Remove deletes it from the batch and closes the composer.
pub fn test_code_review_composer_reopen_and_remove() -> Builder {
    composer_builder(true)
        .with_step(
            open_inline_composer(TEST_FILE_NAME, COMPOSER_LINE)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_inline_composer_open(
                    TEST_FILE_NAME,
                    Some(COMPOSER_LINE),
                )),
        )
        .with_step(
            type_into_inline_composer(TEST_FILE_NAME, "original body")
                .add_assertion(assert_inline_composer_body(TEST_FILE_NAME, "original body")),
        )
        .with_step(
            save_inline_composer(TEST_FILE_NAME)
                .set_timeout(Duration::from_secs(10))
                .add_assertion(assert_inline_composer_closed(TEST_FILE_NAME))
                .add_assertion(assert_saved_comment_count(1)),
        )
        .with_step(
            reopen_saved_inline_comment()
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_inline_composer_open(
                    TEST_FILE_NAME,
                    Some(COMPOSER_LINE),
                ))
                .add_assertion(assert_inline_composer_body(TEST_FILE_NAME, "original body"))
                // The reopened inline BLOCK (resolved through its hosted child, not the composer
                // handle) renders the saved body — proves the block hosts the prefilled editor.
                .add_assertion(assert_inline_comment_block_body(
                    TEST_FILE_NAME,
                    COMPOSER_LINE,
                    "original body",
                ))
                .add_assertion(assert_inline_composer_primary_label(
                    TEST_FILE_NAME,
                    "Update",
                ))
                .add_assertion(assert_inline_composer_shows_remove(TEST_FILE_NAME, true)),
        )
        .with_step(
            remove_inline_comment(TEST_FILE_NAME)
                .set_timeout(Duration::from_secs(10))
                .add_assertion(assert_inline_composer_closed(TEST_FILE_NAME))
                .add_assertion(assert_inline_comment_block_count(TEST_FILE_NAME, 0))
                .add_assertion(assert_saved_comment_count(0)),
        )
}

/// VAL-COMPOSER-011: with the flag OFF, opening the composer uses the floating overlay: NO inline
/// comment block is created (lines below are not pushed down), the floating overlay element IS
/// present at its expected offset, and the line below stays at the no-composer baseline. Asserting
/// the overlay is present (not merely that no inline block exists) guards against a "composer not
/// rendered at all" regression while the flag is off.
pub fn test_code_review_composer_floating_when_flag_off() -> Builder {
    composer_builder(false)
        // Baseline the line below BEFORE opening, so we can prove the overlay does not shift it.
        .with_step(capture_line_below_baseline(TEST_FILE_NAME, COMPOSER_LINE))
        .with_step(
            open_inline_composer(TEST_FILE_NAME, COMPOSER_LINE)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_inline_composer_open(
                    TEST_FILE_NAME,
                    Some(COMPOSER_LINE),
                ))
                .add_assertion(assert_inline_comment_block_count(TEST_FILE_NAME, 0)),
        )
        // The floating overlay must actually paint (regression guard) and not push lines down.
        .with_step(
            TestStep::new(
                "Flag-off composer renders as the floating overlay without shifting lines",
            )
            .set_timeout(Duration::from_secs(10))
            .set_retries(2)
            .add_assertion(assert_floating_overlay_present(TEST_FILE_NAME, true)),
        )
        .with_step(assert_line_below_y_unchanged(TEST_FILE_NAME, COMPOSER_LINE))
}

/// VAL-COMPOSER-016: the composer height tracks the draft. Growing it reflows the line below down by
/// the same delta; deleting lines shrinks the block and reflows the line below back UP by the same
/// delta; and once content exceeds the 200px max-height cap the block height stops growing while the
/// composer becomes internally scrollable.
pub fn test_code_review_composer_height_tracks_content() -> Builder {
    // A body tall enough that its inner content clearly overflows the 200px cap.
    let tall_body = (1..=40)
        .map(|i| format!("draft line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    // An even taller body, to prove the block height holds at the cap rather than growing further.
    let taller_body = (1..=80)
        .map(|i| format!("draft line {i}"))
        .collect::<Vec<_>>()
        .join("\n");

    composer_builder(true)
        .with_step(
            open_inline_composer(TEST_FILE_NAME, COMPOSER_LINE)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_inline_composer_open(
                    TEST_FILE_NAME,
                    Some(COMPOSER_LINE),
                ))
                .add_assertion(assert_inline_composer_pushes_line_below(
                    TEST_FILE_NAME,
                    COMPOSER_LINE,
                )),
        )
        .with_step(
            type_into_inline_composer(TEST_FILE_NAME, "one line")
                .set_timeout(Duration::from_secs(10)),
        )
        .with_step(capture_inline_composer_height(
            TEST_FILE_NAME,
            COMPOSER_LINE,
        ))
        // Grow: adding lines reflows the line below down by the height delta.
        .with_step(
            type_into_inline_composer(TEST_FILE_NAME, "\nline two\nline three\nline four")
                .set_timeout(Duration::from_secs(10)),
        )
        .with_step(
            assert_inline_composer_height_grew(TEST_FILE_NAME, COMPOSER_LINE)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2),
        )
        // Shrink: deleting back to one line reflows the line below back UP by the same delta.
        .with_step(capture_inline_composer_height(
            TEST_FILE_NAME,
            COMPOSER_LINE,
        ))
        .with_step(
            set_inline_composer_body(TEST_FILE_NAME, "one line")
                .set_timeout(Duration::from_secs(10)),
        )
        .with_step(
            assert_inline_composer_height_shrank(TEST_FILE_NAME, COMPOSER_LINE)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2),
        )
        // Cap: content past the cap pins the block height at 200px and scrolls internally.
        .with_step(
            set_inline_composer_body(TEST_FILE_NAME, tall_body)
                .set_timeout(Duration::from_secs(10)),
        )
        .with_step(
            TestStep::new("Composer height pinned at the 200px cap and internally scrollable")
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_inline_composer_height_capped(
                    TEST_FILE_NAME,
                    COMPOSER_LINE,
                )),
        )
        .with_step(capture_inline_composer_height(
            TEST_FILE_NAME,
            COMPOSER_LINE,
        ))
        // Cap enforced: even more content does not grow the block past the cap.
        .with_step(
            set_inline_composer_body(TEST_FILE_NAME, taller_body)
                .set_timeout(Duration::from_secs(10)),
        )
        .with_step(
            assert_inline_composer_height_unchanged(TEST_FILE_NAME, COMPOSER_LINE)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2),
        )
}

/// VAL-COMPOSER-017: with the flag ON, the general/diffset composer stays a header overlay and does
/// NOT create an inline comment block in the diff editor.
pub fn test_code_review_general_composer_stays_overlay() -> Builder {
    composer_builder(true).with_step(
        open_general_composer()
            .set_timeout(Duration::from_secs(10))
            .set_retries(2)
            .add_assertion(assert_general_composer_overlay_present(true))
            .add_assertion(assert_inline_comment_block_count(TEST_FILE_NAME, 0)),
    )
}

/// VAL-COMPOSER-003: the inline composer is part of the in-tree content (a real comment block
/// anchored at its line), not a fixed overlay: after scrolling the code review it stays anchored at
/// the same line and still reserves inline space there (a fixed overlay would detach from the line).
pub fn test_code_review_composer_scrolls_with_line() -> Builder {
    composer_builder(true)
        .with_step(
            open_inline_composer(TEST_FILE_NAME, COMPOSER_LINE)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_inline_composer_open(
                    TEST_FILE_NAME,
                    Some(COMPOSER_LINE),
                ))
                .add_assertion(assert_inline_composer_pushes_line_below(
                    TEST_FILE_NAME,
                    COMPOSER_LINE,
                )),
        )
        .with_step(
            scroll_code_review_to_line(TEST_FILE_NAME, COMPOSER_LINE + 30)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .set_post_step_pause(Duration::from_millis(250)),
        )
        // After scrolling, the composer is still the in-tree block anchored at its line and still
        // reserves inline space (it travelled with the content rather than pinning to the viewport).
        .with_step(
            TestStep::new("Composer stays anchored in-tree after scroll")
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_inline_composer_open(
                    TEST_FILE_NAME,
                    Some(COMPOSER_LINE),
                ))
                .add_assertion(assert_inline_comment_block_count(TEST_FILE_NAME, 1))
                .add_assertion(assert_inline_composer_pushes_line_below(
                    TEST_FILE_NAME,
                    COMPOSER_LINE,
                )),
        )
}

/// VAL-ISOLATION-003: scroll anchoring is preserved while an inline comment block occupies space.
/// Modeled on `test_code_review_scroll_anchor_preserved_when_inserting_above`, with a composer open.
pub fn test_code_review_scroll_anchor_preserved_with_comment_block() -> Builder {
    FeatureFlag::CodeReviewScrollPreservation.set_enabled(true);
    FeatureFlag::IncrementalAutoReload.set_enabled(true);
    FeatureFlag::EmbeddedCodeReviewComments.set_enabled(true);

    let inserted_line_text = inserted_lines("above")
        .into_iter()
        .next()
        .expect("inserted lines should not be empty");

    new_builder()
        .use_tmp_filesystem_for_test_root_directory()
        .with_setup(setup_single_file_repo)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Wait for the terminal to detect the git repository")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_repo_detected()),
        )
        .with_step(
            TestStep::new("Open the code review panel")
                .with_action(|app, window_id, _| open_code_review_panel(app, window_id)),
        )
        .with_step(
            TestStep::new("Wait for the code review panel to load file diffs")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_loaded()),
        )
        // Open a composer near the target so an inline comment block occupies space.
        .with_step(
            open_inline_composer(TEST_FILE_NAME, TARGET_LINE_NUMBER)
                .set_timeout(Duration::from_secs(10))
                .set_retries(2)
                .add_assertion(assert_inline_comment_block_count(TEST_FILE_NAME, 1)),
        )
        .with_step(scroll_code_review_to_target_line())
        .with_step(mutate_test_file(INSERT_ABOVE_LINE_NUMBER, "above"))
        .with_step(
            TestStep::new("Wait for code review to reflect the inserted lines")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_line_text(
                    TEST_FILE_NAME,
                    INSERT_ABOVE_LINE_NUMBER,
                    inserted_line_text,
                )),
        )
        .with_step(
            TestStep::new("Wait for code review to preserve the visible anchor text")
                .set_timeout(Duration::from_secs(20))
                .add_assertion(assert_code_review_anchor(
                    TEST_FILE_NAME,
                    modified_line_text(TARGET_LINE_NUMBER),
                    None,
                )),
        )
}
