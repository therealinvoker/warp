//! TUI view for a `RequestFileEdits` tool call.
//!
//! Unlike stateless tool-call rows, file edits carry state: the view claims the
//! resolved diffs from the shared executor and owns them, then answers the
//! executor's `take_edits` pull at execute time via [`PendingEditsSource`]. The
//! TUI does not yet offer in-place editing, so it hands back the diffs
//! unmodified (no per-file final content) and renders a compact summary.
use warp::tui_export::{
    AIAgentActionId, Appearance, BlocklistAIActionModel, ClaimedEdit, ClaimedEdits,
    DiffSessionType, FileDiff, PendingEditsSource, RequestFileEditsExecutor,
    RequestFileEditsExecutorEvent,
};
use warp_core::ui::theme::Fill as ThemeFill;
use warpui::SingletonEntity;
use warpui_core::elements::tui::{Modifier, TuiContainer, TuiElement, TuiStyle, TuiText};
use warpui_core::elements::Fill;
use warpui_core::{AppContext, Entity, ModelHandle, TuiView, ViewContext, WeakViewHandle};

/// A per-action view backing one `RequestFileEdits` tool call in the transcript.
pub(super) struct TuiFileEditsView {
    action_id: AIAgentActionId,
    executor: ModelHandle<RequestFileEditsExecutor>,
    self_handle: WeakViewHandle<Self>,
    /// The diffs claimed from the executor; `None` until the executor resolves
    /// them (or if this view was created after the action already executed).
    claimed: Option<(Vec<FileDiff>, DiffSessionType)>,
}

impl TuiFileEditsView {
    pub(super) fn new(
        action_id: AIAgentActionId,
        action_model: &ModelHandle<BlocklistAIActionModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let executor = action_model.as_ref(ctx).request_file_edits_executor(ctx);

        // Late `DiffsPrepared` events (preprocess finishing after this view was
        // created) trigger a claim attempt.
        ctx.subscribe_to_model(&executor, |me, _, event, ctx| {
            let RequestFileEditsExecutorEvent::DiffsPrepared(action_id) = event;
            if *action_id == me.action_id {
                me.try_claim(ctx);
            }
        });

        let mut view = Self {
            action_id,
            executor,
            self_handle: ctx.handle(),
            claimed: None,
        };
        // Preprocess may already be done — try to claim immediately.
        view.try_claim(ctx);
        view
    }

    /// Claims the prepared diffs from the executor if not already claimed,
    /// transferring ownership into this view.
    fn try_claim(&mut self, ctx: &mut ViewContext<Self>) {
        if self.claimed.is_some() {
            return;
        }
        let source = Box::new(TuiFileEditsSource(self.self_handle.clone()));
        let action_id = self.action_id.clone();
        let claimed = self.executor.update(ctx, |executor, _| {
            executor.claim_prepared_edits(&action_id, source)
        });
        if let Some(claimed) = claimed {
            self.claimed = Some(claimed);
            ctx.notify();
        }
    }

    /// Total `(files, lines_added, lines_removed)` across the claimed diffs.
    fn summary_stats(&self) -> Option<(usize, usize, usize)> {
        let (diffs, _) = self.claimed.as_ref()?;
        let (added, removed) = diffs
            .iter()
            .map(FileDiff::line_stats)
            .fold((0, 0), |(a, r), (da, dr)| (a + da, r + dr));
        Some((diffs.len(), added, removed))
    }
}

/// Executor-held [`PendingEditsSource`] over a [`TuiFileEditsView`]'s claimed
/// edits. The TUI has no editable buffer yet, so each edit is returned with no
/// per-file final content (persistence applies the diff deltas).
struct TuiFileEditsSource(WeakViewHandle<TuiFileEditsView>);

impl PendingEditsSource for TuiFileEditsSource {
    fn take_edits(&self, app: &AppContext) -> Option<ClaimedEdits> {
        let view = self.0.upgrade(app)?;
        let (diffs, session_type) = view.as_ref(app).claimed.clone()?;
        Some(ClaimedEdits {
            edits: diffs
                .into_iter()
                .map(|diff| ClaimedEdit {
                    diff,
                    final_content: None,
                    was_edited: false,
                })
                .collect(),
            session_type,
        })
    }
}

impl Entity for TuiFileEditsView {
    type Event = ();
}

impl TuiView for TuiFileEditsView {
    fn ui_name() -> &'static str {
        "TuiFileEditsView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let theme = Appearance::as_ref(app).theme();
        let text_color = Fill::from(ThemeFill::from(theme.terminal_colors().bright.black)).into();
        let label = match self.summary_stats() {
            Some((files, added, removed)) => {
                let files_label = if files == 1 { "file" } else { "files" };
                format!("Edited {files} {files_label} (+{added} −{removed})")
            }
            None => "Preparing edits…".to_string(),
        };
        TuiContainer::new(
            TuiText::new(label).with_style(
                TuiStyle::default()
                    .fg(text_color)
                    .add_modifier(Modifier::DIM),
            ),
        )
        .finish()
    }
}
