use warpui::integration::{AssertionCallback, AssertionOutcome, StepDataMap, TestStep};
use warpui::{async_assert, App, ViewHandle, WindowId};

use crate::code_review::code_review_view::{CodeReviewView, CodeReviewVisibleAnchorForTest};

/// Expected scroll region type for assertions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollRegion {
    Header,
    CurrentLine,
    RemovedLine,
    Footer,
}

fn try_single_code_review_view(
    app: &App,
    window_id: WindowId,
) -> Option<ViewHandle<CodeReviewView>> {
    let views = app.views_of_type::<CodeReviewView>(window_id)?;
    if views.len() == 1 {
        Some(views[0].clone())
    } else {
        None
    }
}

fn single_code_review_view(app: &App, window_id: WindowId) -> ViewHandle<CodeReviewView> {
    try_single_code_review_view(app, window_id)
        .expect("expected exactly one code review view in the window")
}

pub fn assert_code_review_loaded() -> AssertionCallback {
    Box::new(|app, window_id| {
        let Some(code_review_view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure(
                "code review view not yet available in the window".to_string(),
            );
        };
        code_review_view.read(app, |code_review_view, _| {
            async_assert!(
                code_review_view.has_file_states()
                    && code_review_view.all_editors_loaded_for_test(),
                "expected code review to have loaded file states and editor buffers"
            )
        })
    })
}

pub fn assert_code_review_anchor(
    expected_file_path: impl Into<String>,
    expected_text: impl Into<String>,
    expected_line_number: Option<usize>,
) -> AssertionCallback {
    let expected_file_path = expected_file_path.into();
    let expected_text = expected_text.into();

    Box::new(move |app, window_id| {
        let Some(code_review_view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure(
                "code review view not yet available in the window".to_string(),
            );
        };
        code_review_view.read(app, |code_review_view, ctx| {
            let Some(anchor) = code_review_view.visible_anchor_for_test(ctx) else {
                return AssertionOutcome::failure(
                    "expected a visible code review anchor but none was available".to_string(),
                );
            };

            assert_anchor(
                &anchor,
                &expected_file_path,
                &expected_text,
                expected_line_number,
            )
        })
    })
}

pub fn scroll_code_review_to_line(file_path: impl Into<String>, line_number: usize) -> TestStep {
    let file_path = file_path.into();

    TestStep::new("Scroll code review to a file line").with_action(move |app, window_id, _| {
        let code_review_view = single_code_review_view(app, window_id);
        code_review_view.update(app, |code_review_view, ctx| {
            let _ = code_review_view.scroll_to_line_for_test(&file_path, line_number, ctx);
        });
    })
}

pub fn assert_code_review_line_text(
    expected_file_path: impl Into<String>,
    line_number: usize,
    expected_text: impl Into<String>,
) -> AssertionCallback {
    let expected_file_path = expected_file_path.into();
    let expected_text = expected_text.into();

    Box::new(move |app, window_id| {
        let Some(code_review_view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure(
                "code review view not yet available in the window".to_string(),
            );
        };
        code_review_view.read(app, |code_review_view, ctx| {
            let Some(line_text) =
                code_review_view.line_text_for_test(&expected_file_path, line_number, ctx)
            else {
                return AssertionOutcome::failure(format!(
                    "expected code review line {line_number} for {expected_file_path:?} to be available",
                ));
            };

            if line_text != expected_text {
                return AssertionOutcome::failure(format!(
                    "expected line {line_number} in {expected_file_path:?} to be {expected_text:?}, got {line_text:?}",
                ));
            }

            AssertionOutcome::Success
        })
    })
}

fn assert_anchor(
    anchor: &CodeReviewVisibleAnchorForTest,
    expected_file_path: &str,
    expected_text: &str,
    expected_line_number: Option<usize>,
) -> AssertionOutcome {
    if anchor.file_path != expected_file_path {
        return AssertionOutcome::failure(format!(
            "expected anchor file to be {expected_file_path:?}, got {:?}",
            anchor.file_path
        ));
    }
    if anchor.line_text != expected_text {
        return AssertionOutcome::failure(format!(
            "expected anchor text to be {expected_text:?}, got {:?}",
            anchor.line_text
        ));
    }
    if let Some(expected_line_number) = expected_line_number {
        if anchor.line_number != expected_line_number {
            return AssertionOutcome::failure(format!(
                "expected anchor line number to be {expected_line_number}, got {}",
                anchor.line_number
            ));
        }
    }

    AssertionOutcome::Success
}

pub fn scroll_code_review_to_header(file_path: impl Into<String>) -> TestStep {
    let file_path = file_path.into();

    TestStep::new("Scroll code review to header region").with_action(move |app, window_id, _| {
        let code_review_view = single_code_review_view(app, window_id);
        code_review_view.update(app, |code_review_view, ctx| {
            code_review_view.scroll_to_header_for_test(&file_path, ctx);
        });
    })
}

pub fn scroll_code_review_to_footer(file_path: impl Into<String>) -> TestStep {
    let file_path = file_path.into();

    TestStep::new("Scroll code review to footer region").with_action(move |app, window_id, _| {
        let code_review_view = single_code_review_view(app, window_id);
        code_review_view.update(app, |code_review_view, ctx| {
            code_review_view.scroll_to_footer_for_test(&file_path, ctx);
        });
    })
}

pub fn scroll_code_review_to_deleted_range(
    file_path: impl Into<String>,
    near_line: usize,
) -> TestStep {
    let file_path = file_path.into();

    TestStep::new("Scroll code review to deleted range").with_action(move |app, window_id, _| {
        let code_review_view = single_code_review_view(app, window_id);
        code_review_view.update(app, |code_review_view, ctx| {
            code_review_view.scroll_to_deleted_range_for_test(&file_path, near_line, ctx);
        });
    })
}

pub fn assert_code_review_scroll_region(expected_region: ScrollRegion) -> AssertionCallback {
    let expected_str = match expected_region {
        ScrollRegion::Header => "header",
        ScrollRegion::CurrentLine => "current_line",
        ScrollRegion::RemovedLine => "removed_line",
        ScrollRegion::Footer => "footer",
    };

    Box::new(move |app, window_id| {
        let Some(code_review_view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure(
                "code review view not yet available in the window".to_string(),
            );
        };
        code_review_view.read(app, |code_review_view, ctx| {
            let actual = code_review_view.scroll_region_for_test(ctx);
            if actual != expected_str {
                return AssertionOutcome::failure(format!(
                    "expected scroll region to be {expected_str:?}, got {actual:?}"
                ));
            }
            AssertionOutcome::Success
        })
    })
}

// --- Inline comment composer drive helpers & assertions ---
//
// These exercise the per-view inline comment composer/blocks (behind
// `FeatureFlag::EmbeddedCodeReviewComments`). Drive helpers run as actions; readers run as polled
// assertions. They delegate to `CodeReviewView`'s `*_for_test` accessors, which resolve the inner
// `CodeEditorView` for a file and read/drive its per-view `RenderState` / composer.

/// Open the inline composer on the given 1-based current line of `file_path`.
pub fn open_inline_composer(file_path: impl Into<String>, line: usize) -> TestStep {
    let file_path = file_path.into();
    TestStep::new("Open inline comment composer").with_action(move |app, window_id, _| {
        let view = single_code_review_view(app, window_id);
        view.update(app, |view, ctx| {
            view.open_comment_line_for_test(&file_path, line, ctx);
        });
    })
}

/// Type `text` into the focused inline composer for `file_path`.
pub fn type_into_inline_composer(
    file_path: impl Into<String>,
    text: impl Into<String>,
) -> TestStep {
    let file_path = file_path.into();
    let text = text.into();
    TestStep::new("Type into inline comment composer").with_action(move |app, window_id, _| {
        let view = single_code_review_view(app, window_id);
        view.update(app, |view, ctx| {
            view.type_into_composer_for_test(&file_path, &text, ctx);
        });
    })
}

/// Save the inline composer for `file_path` via the primary button action.
pub fn save_inline_composer(file_path: impl Into<String>) -> TestStep {
    let file_path = file_path.into();
    TestStep::new("Save inline comment composer").with_action(move |app, window_id, _| {
        let view = single_code_review_view(app, window_id);
        view.update(app, |view, ctx| {
            view.save_composer_for_test(&file_path, ctx);
        });
    })
}

/// Cancel the inline composer for `file_path`.
pub fn cancel_inline_composer(file_path: impl Into<String>) -> TestStep {
    let file_path = file_path.into();
    TestStep::new("Cancel inline comment composer").with_action(move |app, window_id, _| {
        let view = single_code_review_view(app, window_id);
        view.update(app, |view, ctx| {
            view.cancel_composer_for_test(&file_path, ctx);
        });
    })
}

/// Remove the comment currently being edited in the composer for `file_path`.
pub fn remove_inline_comment(file_path: impl Into<String>) -> TestStep {
    let file_path = file_path.into();
    TestStep::new("Remove inline comment").with_action(move |app, window_id, _| {
        let view = single_code_review_view(app, window_id);
        view.update(app, |view, ctx| {
            view.remove_comment_for_test(&file_path, ctx);
        });
    })
}

/// Reopen the first saved line comment as a prefilled inline editor.
pub fn reopen_saved_inline_comment() -> TestStep {
    TestStep::new("Reopen saved inline comment").with_action(move |app, window_id, _| {
        let view = single_code_review_view(app, window_id);
        view.update(app, |view, ctx| {
            view.reopen_saved_comment_for_test(ctx);
        });
    })
}

/// Focus the inline composer's inner text editor for `file_path` (mirrors opening it).
pub fn focus_inline_composer(file_path: impl Into<String>) -> TestStep {
    let file_path = file_path.into();
    TestStep::new("Focus inline composer").with_action(move |app, window_id, _| {
        let view = single_code_review_view(app, window_id);
        view.update(app, |view, ctx| {
            view.focus_composer_for_test(&file_path, ctx);
        });
    })
}

/// Assert whether the inline composer's inner editor for `file_path` holds focus.
pub fn assert_inline_composer_focused(
    file_path: impl Into<String>,
    expected: bool,
) -> AssertionCallback {
    let file_path = file_path.into();
    Box::new(move |app, window_id| {
        let Some(view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure("code review view not yet available".to_string());
        };
        view.read(app, |view, ctx| {
            match view.composer_inner_focused_for_test(&file_path, ctx) {
                Some(focused) if focused == expected => AssertionOutcome::Success,
                Some(focused) => AssertionOutcome::failure(format!(
                    "expected composer focused == {expected}, got {focused}"
                )),
                None => {
                    AssertionOutcome::failure(format!("editor for {file_path:?} not available"))
                }
            }
        })
    })
}

/// Save the inline composer for `file_path` via the Cmd/Ctrl+Enter path.
pub fn cmd_enter_inline_composer(file_path: impl Into<String>) -> TestStep {
    let file_path = file_path.into();
    TestStep::new("Save inline composer via Cmd/Ctrl+Enter").with_action(
        move |app, window_id, _| {
            let view = single_code_review_view(app, window_id);
            view.update(app, |view, ctx| {
                view.save_composer_via_cmd_enter_for_test(&file_path, ctx);
            });
        },
    )
}

/// Press Escape in the inline composer for `file_path` (closes only when the draft is empty).
pub fn escape_inline_composer(file_path: impl Into<String>) -> TestStep {
    let file_path = file_path.into();
    TestStep::new("Press Escape in inline composer").with_action(move |app, window_id, _| {
        let view = single_code_review_view(app, window_id);
        view.update(app, |view, ctx| {
            view.escape_composer_for_test(&file_path, ctx);
        });
    })
}

/// Open the general/diffset (header-anchored) comment composer overlay.
pub fn open_general_composer() -> TestStep {
    TestStep::new("Open general comment composer").with_action(move |app, window_id, _| {
        let view = single_code_review_view(app, window_id);
        view.update(app, |view, ctx| {
            view.open_general_composer_for_test(ctx);
        });
    })
}

/// Assert the inline composer is open for `file_path`; if `expected_line` is `Some`, also assert
/// the anchored line matches.
pub fn assert_inline_composer_open(
    file_path: impl Into<String>,
    expected_line: Option<usize>,
) -> AssertionCallback {
    let file_path = file_path.into();
    Box::new(move |app, window_id| {
        let Some(view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure("code review view not yet available".to_string());
        };
        view.read(app, |view, ctx| {
            let actual = view.composer_open_line_for_test(&file_path, ctx);
            match actual {
                None => AssertionOutcome::failure(
                    "expected the inline composer to be open, but it is closed".to_string(),
                ),
                Some(line) => {
                    if let Some(expected) = expected_line {
                        if line != expected {
                            return AssertionOutcome::failure(format!(
                                "expected composer anchored at line {expected}, got {line}"
                            ));
                        }
                    }
                    AssertionOutcome::Success
                }
            }
        })
    })
}

/// Assert the inline composer is closed for `file_path`.
pub fn assert_inline_composer_closed(file_path: impl Into<String>) -> AssertionCallback {
    let file_path = file_path.into();
    Box::new(move |app, window_id| {
        let Some(view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure("code review view not yet available".to_string());
        };
        view.read(app, |view, ctx| {
            async_assert!(
                !view.composer_open_for_test(&file_path, ctx),
                "expected the inline composer to be closed, but it is open"
            )
        })
    })
}

/// Assert the number of inline comment blocks present for `file_path`.
pub fn assert_inline_comment_block_count(
    file_path: impl Into<String>,
    expected: usize,
) -> AssertionCallback {
    let file_path = file_path.into();
    Box::new(move |app, window_id| {
        let Some(view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure("code review view not yet available".to_string());
        };
        view.read(app, |view, ctx| {
            let actual = view.inline_comment_block_count_for_test(&file_path, ctx);
            match actual {
                Some(count) if count == expected => AssertionOutcome::Success,
                Some(count) => AssertionOutcome::failure(format!(
                    "expected {expected} inline comment block(s) for {file_path:?}, got {count}"
                )),
                None => {
                    AssertionOutcome::failure(format!("editor for {file_path:?} not available"))
                }
            }
        })
    })
}

/// Assert the inline composer's draft body for `file_path` equals `expected`.
pub fn assert_inline_composer_body(
    file_path: impl Into<String>,
    expected: impl Into<String>,
) -> AssertionCallback {
    let file_path = file_path.into();
    let expected = expected.into();
    Box::new(move |app, window_id| {
        let Some(view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure("code review view not yet available".to_string());
        };
        view.read(app, |view, ctx| {
            let actual = view.composer_body_for_test(&file_path, ctx);
            match actual {
                Some(body) if body == expected => AssertionOutcome::Success,
                Some(body) => AssertionOutcome::failure(format!(
                    "expected composer body {expected:?}, got {body:?}"
                )),
                None => {
                    AssertionOutcome::failure(format!("editor for {file_path:?} not available"))
                }
            }
        })
    })
}

/// Assert the inline composer's draft body for `file_path` contains `needle`.
pub fn assert_inline_composer_body_contains(
    file_path: impl Into<String>,
    needle: impl Into<String>,
) -> AssertionCallback {
    let file_path = file_path.into();
    let needle = needle.into();
    Box::new(move |app, window_id| {
        let Some(view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure("code review view not yet available".to_string());
        };
        view.read(app, |view, ctx| {
            match view.composer_body_for_test(&file_path, ctx) {
                Some(body) if body.contains(&needle) => AssertionOutcome::Success,
                Some(body) => AssertionOutcome::failure(format!(
                    "expected composer body to contain {needle:?}, got {body:?}"
                )),
                None => {
                    AssertionOutcome::failure(format!("editor for {file_path:?} not available"))
                }
            }
        })
    })
}

/// Assert whether the inline composer's primary button is disabled for `file_path`.
pub fn assert_inline_composer_save_disabled(
    file_path: impl Into<String>,
    expected_disabled: bool,
) -> AssertionCallback {
    let file_path = file_path.into();
    Box::new(move |app, window_id| {
        let Some(view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure("code review view not yet available".to_string());
        };
        view.read(app, |view, ctx| {
            match view.composer_save_disabled_for_test(&file_path, ctx) {
                Some(disabled) if disabled == expected_disabled => AssertionOutcome::Success,
                Some(disabled) => AssertionOutcome::failure(format!(
                    "expected composer save-disabled == {expected_disabled}, got {disabled}"
                )),
                None => {
                    AssertionOutcome::failure(format!("editor for {file_path:?} not available"))
                }
            }
        })
    })
}

/// Assert the inline composer's primary button label for `file_path`.
pub fn assert_inline_composer_primary_label(
    file_path: impl Into<String>,
    expected: impl Into<String>,
) -> AssertionCallback {
    let file_path = file_path.into();
    let expected = expected.into();
    Box::new(move |app, window_id| {
        let Some(view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure("code review view not yet available".to_string());
        };
        view.read(app, |view, ctx| {
            match view.composer_primary_label_for_test(&file_path, ctx) {
                Some(label) if label == expected => AssertionOutcome::Success,
                Some(label) => AssertionOutcome::failure(format!(
                    "expected composer primary label {expected:?}, got {label:?}"
                )),
                None => {
                    AssertionOutcome::failure(format!("editor for {file_path:?} not available"))
                }
            }
        })
    })
}

/// Assert whether the inline composer shows the "Remove" button for `file_path`.
pub fn assert_inline_composer_shows_remove(
    file_path: impl Into<String>,
    expected: bool,
) -> AssertionCallback {
    let file_path = file_path.into();
    Box::new(move |app, window_id| {
        let Some(view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure("code review view not yet available".to_string());
        };
        view.read(app, |view, ctx| {
            match view.composer_show_remove_for_test(&file_path, ctx) {
                Some(shows) if shows == expected => AssertionOutcome::Success,
                Some(shows) => AssertionOutcome::failure(format!(
                    "expected composer show-remove == {expected}, got {shows}"
                )),
                None => {
                    AssertionOutcome::failure(format!("editor for {file_path:?} not available"))
                }
            }
        })
    })
}

/// Assert the number of saved comments in the active batch.
pub fn assert_saved_comment_count(expected: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let Some(view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure("code review view not yet available".to_string());
        };
        view.read(app, |view, ctx| {
            let actual = view.saved_comment_count_for_test(ctx);
            async_assert!(
                actual == expected,
                "expected {expected} saved comment(s), got {actual}"
            )
        })
    })
}

/// Assert whether the general/diffset composer overlay is present.
pub fn assert_general_composer_overlay_present(expected: bool) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let Some(view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure("code review view not yet available".to_string());
        };
        view.read(app, |view, _ctx| {
            let actual = view.general_composer_overlay_present_for_test();
            async_assert!(
                actual == expected,
                "expected general composer overlay present == {expected}, got {actual}"
            )
        })
    })
}

/// Assert that opening the inline composer on `line` of `file_path` pushed the display line below it
/// down by at least the composer's reserved height (with no overlap): the gap between line `line`
/// and line `line + 1` must exceed the composer block height (which must be > 0).
pub fn assert_inline_composer_pushes_line_below(
    file_path: impl Into<String>,
    line: usize,
) -> AssertionCallback {
    let file_path = file_path.into();
    Box::new(move |app, window_id| {
        let Some(view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure("code review view not yet available".to_string());
        };
        view.read(app, |view, ctx| {
            let Some(block_height) =
                view.comment_block_height_for_test(&file_path, line, ctx)
            else {
                return AssertionOutcome::failure(
                    "expected an inline composer block but none was present".to_string(),
                );
            };
            if block_height <= 0.0 {
                return AssertionOutcome::failure(format!(
                    "expected composer block height > 0, got {block_height}"
                ));
            }
            let Some(line_y) = view.line_viewport_y_for_test(&file_path, line, ctx) else {
                return AssertionOutcome::failure("line Y not available".to_string());
            };
            let Some(line_below_y) =
                view.line_viewport_y_for_test(&file_path, line + 1, ctx)
            else {
                return AssertionOutcome::failure("line-below Y not available".to_string());
            };
            let block_y = view
                .comment_block_viewport_y_for_test(&file_path, line, ctx)
                .unwrap_or(f32::NAN);
            // The inline block occupies real vertical space between the anchored line and the next
            // display line: the next line must start at or after the block's bottom (no overlap).
            let block_bottom = block_y + block_height;
            if line_below_y + 1.0 >= block_bottom {
                AssertionOutcome::Success
            } else {
                AssertionOutcome::failure(format!(
                    "line {} overlaps composer block: line_y={line_y}, block_y={block_y}, block_height={block_height}, block_bottom={block_bottom}, line_below_y={line_below_y}",
                    line + 1
                ))
            }
        })
    })
}

/// Assert the inline comment block anchored at `line` of `file_path` renders `expected` as its body
/// text, resolved through the block's hosted child (not the composer handle directly).
pub fn assert_inline_comment_block_body(
    file_path: impl Into<String>,
    line: usize,
    expected: impl Into<String>,
) -> AssertionCallback {
    let file_path = file_path.into();
    let expected = expected.into();
    Box::new(move |app, window_id| {
        let Some(view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure("code review view not yet available".to_string());
        };
        view.read(app, |view, ctx| {
            match view.inline_comment_block_body_for_test(&file_path, line, ctx) {
                Some(body) if body == expected => AssertionOutcome::Success,
                Some(body) => AssertionOutcome::failure(format!(
                    "expected inline comment block body {expected:?} at line {line}, got {body:?}"
                )),
                None => AssertionOutcome::failure(format!(
                    "expected an inline comment block at line {line} for {file_path:?}, none present"
                )),
            }
        })
    })
}

/// Replace the active inline composer's draft body for `file_path` (mirrors deleting/retyping
/// lines, used to shrink the composer back down).
pub fn set_inline_composer_body(file_path: impl Into<String>, text: impl Into<String>) -> TestStep {
    let file_path = file_path.into();
    let text = text.into();
    TestStep::new("Replace inline composer body").with_action(move |app, window_id, _| {
        let view = single_code_review_view(app, window_id);
        view.update(app, |view, ctx| {
            view.set_composer_body_for_test(&file_path, &text, ctx);
        });
    })
}

/// Assert whether the flag-OFF floating composer overlay actually painted for `file_path` and is
/// anchored at a well-defined viewport offset (so a "composer not rendered at all" regression
/// fails). `expected_present` is whether the overlay should be present.
pub fn assert_floating_overlay_present(
    file_path: impl Into<String>,
    expected_present: bool,
) -> AssertionCallback {
    let file_path = file_path.into();
    Box::new(move |app, window_id| {
        let Some(view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure("code review view not yet available".to_string());
        };
        view.read(app, |view, ctx| {
            let present = match view.floating_overlay_present_for_test(&file_path, ctx) {
                Some(present) => present,
                None => {
                    return AssertionOutcome::failure(format!(
                        "editor for {file_path:?} not available"
                    ))
                }
            };
            if present != expected_present {
                return AssertionOutcome::failure(format!(
                    "expected floating overlay present == {expected_present}, got {present}"
                ));
            }
            if expected_present {
                match view.floating_overlay_offset_for_test(&file_path, ctx) {
                    Some(offset) if offset > 0.0 => AssertionOutcome::Success,
                    Some(offset) => AssertionOutcome::failure(format!(
                        "expected floating overlay anchored at a positive offset, got {offset}"
                    )),
                    None => AssertionOutcome::failure(
                        "expected a floating overlay anchor offset, but no composer is open"
                            .to_string(),
                    ),
                }
            } else {
                AssertionOutcome::Success
            }
        })
    })
}

const COMPOSER_BLOCK_HEIGHT_KEY: &str = "inline_composer_block_height";
const COMPOSER_LINE_BELOW_Y_KEY: &str = "inline_composer_line_below_y";

/// Capture the inline composer block's reserved height and the on-screen Y of the line below it
/// into step data, for a later [`assert_inline_composer_height_grew`] check.
pub fn capture_inline_composer_height(file_path: impl Into<String>, line: usize) -> TestStep {
    let file_path = file_path.into();
    TestStep::new("Capture inline composer height").with_action(
        move |app: &mut App, window_id: WindowId, step_data: &mut StepDataMap| {
            let view = single_code_review_view(app, window_id);
            view.read(app, |view, ctx| {
                if let (Some(height), Some(line_below_y)) = (
                    view.comment_block_height_for_test(&file_path, line, ctx),
                    view.line_viewport_y_for_test(&file_path, line + 1, ctx),
                ) {
                    step_data.insert(COMPOSER_BLOCK_HEIGHT_KEY, height);
                    step_data.insert(COMPOSER_LINE_BELOW_Y_KEY, line_below_y);
                }
            });
        },
    )
}

/// Assert the inline composer block grew taller than the captured height and that the line below it
/// shifted down by the same delta (within 2px) — i.e. growing the draft reflows the lines below.
pub fn assert_inline_composer_height_grew(file_path: impl Into<String>, line: usize) -> TestStep {
    let file_path = file_path.into();
    TestStep::new("Assert inline composer grew and reflowed lines below")
        .add_named_assertion_with_data_from_prior_step(
            "composer block height and line-below shift grow together",
            move |app: &mut App, window_id: WindowId, step_data: &mut StepDataMap| {
                let Some(view) = try_single_code_review_view(app, window_id) else {
                    return AssertionOutcome::failure(
                        "code review view not yet available".to_string(),
                    );
                };
                let Some(&prior_height) = step_data.get::<_, f32>(COMPOSER_BLOCK_HEIGHT_KEY) else {
                    return AssertionOutcome::failure(
                        "no captured composer height from a prior step".to_string(),
                    );
                };
                let Some(&prior_line_below_y) =
                    step_data.get::<_, f32>(COMPOSER_LINE_BELOW_Y_KEY)
                else {
                    return AssertionOutcome::failure(
                        "no captured line-below Y from a prior step".to_string(),
                    );
                };
                view.read(app, |view, ctx| {
                    let Some(height) = view.comment_block_height_for_test(&file_path, line, ctx)
                    else {
                        return AssertionOutcome::failure("composer height not available".to_string());
                    };
                    let Some(line_below_y) =
                        view.line_viewport_y_for_test(&file_path, line + 1, ctx)
                    else {
                        return AssertionOutcome::failure("line-below Y not available".to_string());
                    };
                    let height_delta = height - prior_height;
                    let shift_delta = line_below_y - prior_line_below_y;
                    if height_delta <= 1.0 {
                        return AssertionOutcome::failure(format!(
                            "expected composer block to grow, but height went {prior_height} -> {height}"
                        ));
                    }
                    if (height_delta - shift_delta).abs() > 2.0 {
                        return AssertionOutcome::failure(format!(
                            "expected line below to shift by the height delta {height_delta}, but it shifted {shift_delta}"
                        ));
                    }
                    AssertionOutcome::Success
                })
            },
        )
}

/// Assert the inline composer block shrank below the captured height and that the line below it
/// shifted back UP by the same delta (within 2px) — i.e. deleting draft lines reflows the lines
/// below back up. Pairs with a prior [`capture_inline_composer_height`].
pub fn assert_inline_composer_height_shrank(file_path: impl Into<String>, line: usize) -> TestStep {
    let file_path = file_path.into();
    TestStep::new("Assert inline composer shrank and reflowed lines back up")
        .add_named_assertion_with_data_from_prior_step(
            "composer block height and line-below shift shrink together",
            move |app: &mut App, window_id: WindowId, step_data: &mut StepDataMap| {
                let Some(view) = try_single_code_review_view(app, window_id) else {
                    return AssertionOutcome::failure(
                        "code review view not yet available".to_string(),
                    );
                };
                let Some(&prior_height) = step_data.get::<_, f32>(COMPOSER_BLOCK_HEIGHT_KEY) else {
                    return AssertionOutcome::failure(
                        "no captured composer height from a prior step".to_string(),
                    );
                };
                let Some(&prior_line_below_y) =
                    step_data.get::<_, f32>(COMPOSER_LINE_BELOW_Y_KEY)
                else {
                    return AssertionOutcome::failure(
                        "no captured line-below Y from a prior step".to_string(),
                    );
                };
                view.read(app, |view, ctx| {
                    let Some(height) = view.comment_block_height_for_test(&file_path, line, ctx)
                    else {
                        return AssertionOutcome::failure("composer height not available".to_string());
                    };
                    let Some(line_below_y) =
                        view.line_viewport_y_for_test(&file_path, line + 1, ctx)
                    else {
                        return AssertionOutcome::failure("line-below Y not available".to_string());
                    };
                    let height_delta = prior_height - height;
                    let shift_delta = prior_line_below_y - line_below_y;
                    if height_delta <= 1.0 {
                        return AssertionOutcome::failure(format!(
                            "expected composer block to shrink, but height went {prior_height} -> {height}"
                        ));
                    }
                    if (height_delta - shift_delta).abs() > 2.0 {
                        return AssertionOutcome::failure(format!(
                            "expected line below to shift back up by the height delta {height_delta}, but it shifted {shift_delta}"
                        ));
                    }
                    AssertionOutcome::Success
                })
            },
        )
}

const COMPOSER_MAX_HEIGHT: f32 = 200.0;

/// Assert the inline composer is pinned at the 200px max-height cap and is internally scrollable:
/// the reserved block height equals the cap (within 2px), the composer reports it is at the cap,
/// and the inner content height overflows the reserved block (so it scrolls internally rather than
/// growing). Use a body tall enough that the inner content clearly exceeds the cap.
pub fn assert_inline_composer_height_capped(
    file_path: impl Into<String>,
    line: usize,
) -> AssertionCallback {
    let file_path = file_path.into();
    Box::new(move |app, window_id| {
        let Some(view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure("code review view not yet available".to_string());
        };
        view.read(app, |view, ctx| {
            let Some(height) = view.comment_block_height_for_test(&file_path, line, ctx) else {
                return AssertionOutcome::failure("composer height not available".to_string());
            };
            if (height - COMPOSER_MAX_HEIGHT).abs() > 2.0 {
                return AssertionOutcome::failure(format!(
                    "expected composer block height pinned at {COMPOSER_MAX_HEIGHT}px cap, got {height}"
                ));
            }
            match view.composer_at_max_height_for_test(&file_path, ctx) {
                Some(true) => {}
                Some(false) => {
                    return AssertionOutcome::failure(
                        "expected composer to report it is at the max-height cap".to_string(),
                    )
                }
                None => {
                    return AssertionOutcome::failure(format!(
                        "editor for {file_path:?} not available"
                    ))
                }
            }
            let Some(inner_height) =
                view.composer_inner_content_height_for_test(&file_path, ctx)
            else {
                return AssertionOutcome::failure("composer inner height not available".to_string());
            };
            if inner_height <= height {
                return AssertionOutcome::failure(format!(
                    "expected inner content ({inner_height}) to overflow the capped block ({height}) so the composer scrolls internally"
                ));
            }
            AssertionOutcome::Success
        })
    })
}

/// Assert the inline composer block height has NOT changed beyond 2px from the captured height —
/// used after adding still more content past the cap to prove the height stops growing. Pairs with
/// a prior [`capture_inline_composer_height`].
pub fn assert_inline_composer_height_unchanged(
    file_path: impl Into<String>,
    line: usize,
) -> TestStep {
    let file_path = file_path.into();
    TestStep::new("Assert inline composer height held at the cap")
        .add_named_assertion_with_data_from_prior_step(
            "composer block height unchanged past the cap",
            move |app: &mut App, window_id: WindowId, step_data: &mut StepDataMap| {
                let Some(view) = try_single_code_review_view(app, window_id) else {
                    return AssertionOutcome::failure(
                        "code review view not yet available".to_string(),
                    );
                };
                let Some(&prior_height) = step_data.get::<_, f32>(COMPOSER_BLOCK_HEIGHT_KEY) else {
                    return AssertionOutcome::failure(
                        "no captured composer height from a prior step".to_string(),
                    );
                };
                view.read(app, |view, ctx| {
                    let Some(height) = view.comment_block_height_for_test(&file_path, line, ctx)
                    else {
                        return AssertionOutcome::failure("composer height not available".to_string());
                    };
                    if (height - prior_height).abs() > 2.0 {
                        return AssertionOutcome::failure(format!(
                            "expected composer block height to hold at the cap, but it went {prior_height} -> {height}"
                        ));
                    }
                    AssertionOutcome::Success
                })
            },
        )
}

const LINE_BELOW_BASELINE_Y_KEY: &str = "inline_composer_line_below_baseline_y";

/// Capture the on-screen Y of the line below `line` of `file_path` into step data, to later assert
/// it is unchanged (the flag-OFF floating overlay must not push lines down).
pub fn capture_line_below_baseline(file_path: impl Into<String>, line: usize) -> TestStep {
    let file_path = file_path.into();
    TestStep::new("Capture line-below baseline Y").with_action(
        move |app: &mut App, window_id: WindowId, step_data: &mut StepDataMap| {
            let view = single_code_review_view(app, window_id);
            view.read(app, |view, ctx| {
                if let Some(line_below_y) = view.line_viewport_y_for_test(&file_path, line + 1, ctx)
                {
                    step_data.insert(LINE_BELOW_BASELINE_Y_KEY, line_below_y);
                }
            });
        },
    )
}

/// Assert the on-screen Y of the line below `line` equals the captured baseline (within 1px) — used
/// to prove the flag-OFF floating overlay does NOT shift the lines below it down.
pub fn assert_line_below_y_unchanged(file_path: impl Into<String>, line: usize) -> TestStep {
    let file_path = file_path.into();
    TestStep::new("Assert line-below Y unchanged from baseline")
        .add_named_assertion_with_data_from_prior_step(
            "line below the composer stays at the no-composer baseline",
            move |app: &mut App, window_id: WindowId, step_data: &mut StepDataMap| {
                let Some(view) = try_single_code_review_view(app, window_id) else {
                    return AssertionOutcome::failure(
                        "code review view not yet available".to_string(),
                    );
                };
                let Some(&baseline_y) = step_data.get::<_, f32>(LINE_BELOW_BASELINE_Y_KEY) else {
                    return AssertionOutcome::failure(
                        "no captured line-below baseline Y from a prior step".to_string(),
                    );
                };
                view.read(app, |view, ctx| {
                    let Some(line_below_y) =
                        view.line_viewport_y_for_test(&file_path, line + 1, ctx)
                    else {
                        return AssertionOutcome::failure("line-below Y not available".to_string());
                    };
                    if (line_below_y - baseline_y).abs() > 1.0 {
                        return AssertionOutcome::failure(format!(
                            "expected line below to stay at baseline {baseline_y}, but it moved to {line_below_y}"
                        ));
                    }
                    AssertionOutcome::Success
                })
            },
        )
}
