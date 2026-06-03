//! Integration/test-only accessors and drive helpers for [`CodeEditorView`]'s inline code-review
//! comment surfaces (the composer and, later, saved inline cards). These are reused by the
//! code-review integration tests via [`crate::code_review::CodeReviewView`] and by F4/F5
//! saved-comment coverage.

use warp_editor::model::CoreEditorModel;
use warp_editor::render::model::{LineCount, RenderLineLocation};
use warpui::{AppContext, TypedActionView, ViewContext};

use super::{CodeEditorView, CodeEditorViewAction};
use crate::code::editor::comment_editor::CommentEditorAction;
use crate::code::editor::comments::PendingComment;
use crate::code::editor::embedded_comment::LaidOutEmbeddedCommentSpace;
use crate::code::editor::line::EditorLineLocation;

fn current_line_location(line_number: usize) -> EditorLineLocation {
    let line_number = LineCount::from(line_number);
    EditorLineLocation::Current {
        line_number,
        line_range: line_number..line_number + LineCount::from(1),
    }
}

impl CodeEditorView {
    /// The 1-based current line the inline composer is open at, or `None` when no composer is open.
    pub fn composer_open_line_for_test(&self, app: &AppContext) -> Option<usize> {
        match &self
            .model
            .as_ref(app)
            .comments()
            .as_ref(app)
            .pending_comment
        {
            PendingComment::Open { line } => line.line_number().map(|lc| lc.as_u32() as usize),
            PendingComment::Closed => None,
        }
    }

    /// The current draft/body text of the active composer.
    pub fn composer_body_for_test(&self, app: &AppContext) -> String {
        self.active_comment_editor.as_ref(app).comment_text(app)
    }

    /// Whether the composer's inner text editor currently holds focus.
    pub fn composer_inner_focused_for_test(&self, app: &AppContext) -> bool {
        self.active_comment_editor
            .as_ref(app)
            .inner_editor_focused_for_test(app)
    }

    /// Focus the composer's inner text editor (mirrors what opening the composer does).
    pub fn focus_composer_for_test(&mut self, ctx: &mut ViewContext<Self>) {
        self.active_comment_editor.update(ctx, |composer, ctx| {
            composer.focus_inner_editor_for_test(ctx);
        });
    }

    /// Whether the composer's primary ("Comment"/"Update") button is disabled (empty draft).
    pub fn composer_save_disabled_for_test(&self, app: &AppContext) -> bool {
        self.active_comment_editor
            .as_ref(app)
            .save_button_disabled_for_test()
    }

    /// The label of the composer's primary button ("Comment" for new, "Update" when editing).
    pub fn composer_primary_label_for_test(&self, app: &AppContext) -> String {
        self.active_comment_editor
            .as_ref(app)
            .primary_button_label_for_test(app)
    }

    /// Whether the composer shows the "Remove" button (true when editing an existing comment).
    pub fn composer_show_remove_for_test(&self, app: &AppContext) -> bool {
        self.active_comment_editor
            .as_ref(app)
            .show_remove_button_for_test()
    }

    /// Number of inline comment blocks in this view's per-view render state.
    pub fn inline_comment_block_count_for_test(&self, app: &AppContext) -> usize {
        self.model
            .as_ref(app)
            .render_state()
            .as_ref(app)
            .comment_block_count()
    }

    /// On-screen (viewport-space) Y of the top of the given 1-based current line, or `None` if the
    /// line is not laid out.
    pub fn line_viewport_y_for_test(&self, line: usize, app: &AppContext) -> Option<f32> {
        self.model
            .as_ref(app)
            .render_state()
            .as_ref(app)
            .vertical_offset_at_render_location(RenderLineLocation::Current(LineCount::from(line)))
            .map(|p| p.as_f32())
    }

    /// On-screen (viewport-space) Y of the top of the inline comment block anchored at the given
    /// 1-based current line, or `None` if no block is anchored there.
    pub fn comment_block_viewport_y_for_test(&self, line: usize, app: &AppContext) -> Option<f32> {
        let render_state = self.model.as_ref(app).render_state();
        let render_state = render_state.as_ref(app);
        render_state
            .comment_block_position(RenderLineLocation::Current(LineCount::from(line)))
            .map(|position| {
                (position.start_y_offset - render_state.viewport().scroll_top()).as_f32()
            })
    }

    /// Reserved height (content-space) of the inline comment block anchored at the given 1-based
    /// current line, or `None` if no block is anchored there.
    pub fn comment_block_height_for_test(&self, line: usize, app: &AppContext) -> Option<f32> {
        self.model
            .as_ref(app)
            .render_state()
            .as_ref(app)
            .comment_block_position(RenderLineLocation::Current(LineCount::from(line)))
            .map(|position| position.content_height.as_f32())
    }

    /// Open the inline composer on the given 1-based current line (mirrors a gutter add-comment
    /// click / `NewCommentOnLine`).
    pub fn open_comment_line_for_test(&mut self, line: usize, ctx: &mut ViewContext<Self>) {
        self.handle_action(
            &CodeEditorViewAction::NewCommentOnLine {
                line: current_line_location(line),
            },
            ctx,
        );
    }

    /// Type `text` into the focused composer (appends to the current draft).
    pub fn type_into_composer_for_test(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        self.active_comment_editor.update(ctx, |composer, ctx| {
            composer.type_text_for_test(text, ctx);
        });
    }

    /// Invoke the composer's primary save action (equivalent to clicking "Comment"/"Update").
    pub fn save_composer_for_test(&mut self, ctx: &mut ViewContext<Self>) {
        self.active_comment_editor.update(ctx, |composer, ctx| {
            composer.handle_action(&CommentEditorAction::SaveComment, ctx);
        });
    }

    /// Cancel the composer (equivalent to clicking "Cancel").
    pub fn cancel_composer_for_test(&mut self, ctx: &mut ViewContext<Self>) {
        self.active_comment_editor.update(ctx, |composer, ctx| {
            composer.handle_action(&CommentEditorAction::CloseEditor, ctx);
        });
    }

    /// Save the composer via the Cmd/Ctrl+Enter path.
    pub fn save_composer_via_cmd_enter_for_test(&mut self, ctx: &mut ViewContext<Self>) {
        self.active_comment_editor.update(ctx, |composer, ctx| {
            composer.cmd_enter_for_test(ctx);
        });
    }

    /// Press Escape in the composer (closes only when the draft is empty).
    pub fn escape_composer_for_test(&mut self, ctx: &mut ViewContext<Self>) {
        self.active_comment_editor.update(ctx, |composer, ctx| {
            composer.escape_for_test(ctx);
        });
    }

    /// Remove the comment currently being edited (equivalent to clicking "Remove").
    pub fn remove_comment_for_test(&mut self, ctx: &mut ViewContext<Self>) {
        self.active_comment_editor.update(ctx, |composer, ctx| {
            composer.handle_action(&CommentEditorAction::RemoveComment, ctx);
        });
    }

    /// The rendered body text of the inline comment block anchored at the given 1-based current
    /// line, resolved through the block's hosted child (not the view's composer handle directly),
    /// or `None` if no inline comment block is anchored there. Reused by F4/F5 saved-comment tests.
    pub fn inline_comment_block_body_for_test(
        &self,
        line: usize,
        app: &AppContext,
    ) -> Option<String> {
        let item = self
            .model
            .as_ref(app)
            .render_state()
            .as_ref(app)
            .comment_block_item(RenderLineLocation::Current(LineCount::from(line)))?;
        item.as_any()
            .downcast_ref::<LaidOutEmbeddedCommentSpace>()?
            .rendered_body_for_test(app)
    }

    /// Replace the active composer's draft body with `text` (mirrors deleting/retyping lines).
    pub fn set_composer_body_for_test(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        self.active_comment_editor.update(ctx, |composer, ctx| {
            composer.set_body_for_test(text, ctx);
        });
    }

    /// The active composer's inner markdown editor full content height (independent of the 200px
    /// max-height cap). Greater than the reserved block height implies internal scrolling.
    pub fn composer_inner_content_height_for_test(&self, app: &AppContext) -> f32 {
        self.active_comment_editor
            .as_ref(app)
            .inner_content_height_for_test(app)
    }

    /// Whether the active composer's reserved inline height is pinned at the 200px max-height cap.
    pub fn composer_at_max_height_for_test(&self, app: &AppContext) -> bool {
        self.active_comment_editor
            .as_ref(app)
            .is_at_max_height_for_test(app)
    }

    /// Whether the flag-OFF floating comment composer overlay actually painted on the previous
    /// frame (its element position was recorded). This is `false` when the overlay branch did not
    /// render at all, so it guards against a "composer not rendered" regression while the flag is
    /// off.
    pub fn floating_overlay_present_for_test(&self, app: &AppContext) -> bool {
        app.element_position_by_id_at_last_frame(self.window_id, &self.comment_overlay_position_id)
            .is_some()
    }

    /// The viewport-space Y offset at which the flag-OFF floating composer overlay is anchored
    /// (the anchored line's offset plus one line height), or `None` when no composer is open. This
    /// mirrors the offset `render` positions the overlay at.
    pub fn floating_overlay_offset_for_test(&self, app: &AppContext) -> Option<f32> {
        let line = match &self
            .model
            .as_ref(app)
            .comments()
            .as_ref(app)
            .pending_comment
        {
            PendingComment::Open { line } => line.clone(),
            PendingComment::Closed => return None,
        };
        let render_state = self.model.as_ref(app).render_state();
        let render_state = render_state.as_ref(app);
        let offset = render_state
            .vertical_offset_at_render_location(line.into_render_line_location())?
            + render_state.styles().base_line_height();
        Some(offset.as_f32())
    }
}
