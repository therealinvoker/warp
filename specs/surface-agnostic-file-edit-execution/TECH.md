# Surface-Agnostic File-Edit Execution TECH
## Context
This branch is stacked on `tui-agent-tool-calling`, which renders `RequestFileEdits` in the transcript like any other tool call but leaves it unexecuted on non-GUI surfaces: `RequestFileEditsExecutor::execute` requires a registered GUI `CodeDiffView` and returns `NotReady` otherwise (`app/src/ai/blocklist/action_model/execute/request_file_edits.rs`).
The executor is a shared tool executor that both the GUI terminal and the TUI drive, but it is coupled to the GUI in three ways:
- It stores `diff_views: HashMap<AIAgentActionId, ViewHandle<CodeDiffView>>` and drives that view to save files and read back the result (`request_file_edits.rs`).
- The GUI `CodeDiffView` is the only producer of a successful `RequestFileEditsResult`, assembled in `try_emit_diffs_saved` and emitted as `CodeDiffViewEvent::SavedAcceptedDiffs` (`app/src/ai/blocklist/inline_action/code_diff_view.rs`), which the executor repackages.
- The diff data types `FileDiff`, `DiffBase`, and `DiffSessionType` are defined inside the GUI view module `code_diff_view.rs`.
`RequestFileEdits` has a two-phase lifecycle: `preprocess_action` resolves the LLM's edits into concrete diffs (async file reads via `ApplyDiffModel::apply_diffs`) and `execute` runs later to persist them. State computed in preprocess must survive an arbitrary user-interaction gap. Today the `CodeDiffView` is that survivor — its editor buffers hold the resolved (and possibly user-edited) diffs and produce the result on accept.
Relevant code:
- `app/src/ai/blocklist/action_model/execute/request_file_edits.rs` — executor: `diff_views`, `diff_application_failures`, `execute`, `on_diffs_applied`, `should_autoexecute`, `register_requested_edits`, and the shared `updated_file_contexts_from_editor_buffers` helper.
- `app/src/ai/blocklist/action_model/execute/request_file_edits/apply_diff_model.rs` — the GUI-less resolve-side submodel pattern this refactor mirrors for the persist side.
- `app/src/ai/blocklist/inline_action/code_diff_view.rs` — `CodeDiffView`, the `FileDiff`/`DiffBase`/`DiffSessionType` definitions, `set_candidate_diffs`, `accept_and_save`, `SavingDiffs`/`SaveStatus`/`DiffApplicationState`, `handle_save_completed`, `accepted_file_diff_computed`, `try_emit_diffs_saved`, and the `SavedAcceptedDiffs` event.
- `app/src/code/inline_diff.rs` — `InlineDiffView::register_file`, `save_content`, `accept_and_save_diff`, `restore_diff_base`.
- `app/src/ai/blocklist/block.rs` — `handle_requested_edit_complete` creates the `CodeDiffView`, calls `register_requested_edits`, subscribes to `CodeDiffViewEvent`, and on `TryAccept` calls `action_model.execute_action`.
- `app/src/terminal/view.rs` — `on_maa_code_diff_generated` builds a `CodeDiffView::new_passive` and self-drives `accept_and_save` + `SavedAcceptedDiffs` for passive code-diff suggestions, independent of the executor.
- `crates/warp_files/src/lib.rs` — `FileModel::save` / `rename_and_save` / `delete` route local vs. remote and emit `FileSaved` / `FailedToSave`; registration via `register_file_path` / `register_remote_file`.
## Goal
Make file-edit tool calls executable on any surface (GUI and TUI/headless) by routing all persistence through one shared, non-GUI model, and remove the executor's GUI coupling. The interactive review surface — the only thing that genuinely differs between GUI and TUI — owns the diffs while they are under review; the executor knows surfaces only through the `PendingEditsSource` trait and receives plain `ClaimedEdits` data at execute time.
## Proposed changes
### Two focused models
Keep `ApplyDiffModel` as-is — it remains the resolve/read-and-compute half (`apply_diffs` -> `Vec<AIRequestedCodeDiff>`). Add `PersistDiffModel` in `app/src/ai/blocklist/persist_diff_model.rs` (a sibling of `diff_types.rs`, so no GUI or executor-internal module has to re-export it) as the single writer + result producer for every surface. It is a `SingletonEntity` registered at app init immediately after `FileModel` (`app/src/lib.rs`), matching the ownership model of the singleton it wraps; call sites reach it via `PersistDiffModel::handle(ctx)`.
### PersistDiffModel
One shared entry point used by every surface:
```rust
pub(crate) fn resolve_and_persist(
    &mut self,
    claimed: ClaimedEdits,
    ctx: &mut ModelContext<Self>,
) -> BoxFuture<'static, RequestFileEditsResult>
```
Resolution and persistence live behind this one function so no caller hand-assembles resolved edits. `ClaimedEdits` (defined in `diff_types.rs`) is the execute-time snapshot: `edits: Vec<ClaimedEdit>` plus the `session_type`, where each `ClaimedEdit` pairs a `FileDiff` with `final_content: Option<String>`. Resolution uses `final_content` (review-surface-supplied, possibly user-edited) when present, otherwise applies the diff's deltas to the base content (`build_resolved_edits` / `apply_deltas_to_content` / `split_lines_preserving_newlines`, private to the module) — so headless and reviewed edits differ only in whether `final_content` is set, never in code path. Persistence then, per file, decides the actual write operation exactly once via a `PersistAction` enum derived from `(op, session_type)` — `Write`, `Rename(PathBuf)` (local sessions only; remote has no rename primitive and falls back to an in-place write at the original path), or `Delete` — and drives **both** the `FileModel` dispatch (`save` / `rename_and_save` / `delete`) and the reported outcome (updated/deleted paths, `similar`-based `diff_result`) from that one value, so the report can never diverge from the write. It registers each file with `FileModel` (`register_file_path` local / `register_remote_file` remote), tracks completion via `FileModelEvent` keyed by `FileId`, and calls `FileModel::unsubscribe` for each file once its save/delete resolves (or dispatch fails) so `FileModel` state does not grow unboundedly. When all files resolve it assembles `RequestFileEditsResult`, mapping any `FailedToSave` -> `DiffApplicationFailed`; `updated_files` is built via `updated_file_contexts_from_content_map` (moved here from the executor and renamed from `updated_file_contexts_from_editor_buffers`).
Multiple `resolve_and_persist` calls can be in flight concurrently (e.g. edits executing from different conversations): each call tracks its own batch of outstanding `FileId`s and resolves independently, and `FileModel` mints a fresh `FileId` per registration, so completion events are attributed to the correct batch even when two batches target the same path.
The pure changed-lines helpers (`changed_lines_from_op`, `changed_line_range_for_delta`, `inserted_content_range`) live in `diff_types.rs`, shared by the persist model and the GUI telemetry path instead of duplicated per surface.
### Executor becomes surface-agnostic
Governing invariant: **prepared file content has exactly one owner at any time**. File content under review must not be duplicated between the executor and the review surface — the surface's buffers are the sole resident copy while the user reviews. The executor therefore never retains diff data after a surface claims it; it keeps only a handle it can pull the final state back through at execute time:
```rust
/// A review surface's handle over pending edits it has claimed. Consulted
/// exactly once, at execute time, to obtain the final (possibly user-edited)
/// state of the edits.
pub trait PendingEditsSource {
    fn take_edits(&self, app: &AppContext) -> Option<ClaimedEdits>;
}

enum PendingFileEdits {
    /// Diffs resolved; no review surface has claimed them yet (executor owns).
    Unclaimed { diffs: Vec<FileDiff>, session_type: DiffSessionType },
    /// A review surface owns the diffs; `execute` pulls the final state back.
    Claimed(Box<dyn PendingEditsSource>),
    /// Diff application failed during preprocess; `execute` reports it.
    Failed(Vec1<DiffApplicationError>),
}
```
Executor fields become: `active_session`, `apply_diff_model`, `pending_file_edits: HashMap<AIAgentActionId, PendingFileEdits>`, `terminal_view_id` — replacing `diff_views` and `diff_application_failures` (the persist model is a singleton, not an executor field). `register_requested_edits` is removed.
- `on_diffs_applied`: insert `Unclaimed { diffs, session_type }` on success, or `Failed(errors)` on failure; no view calls. Emits `RequestFileEditsExecutorEvent::DiffsPrepared(action_id)` for review surfaces.
- `claim_prepared_edits(action_id, source)`: transfers ownership of an `Unclaimed` entry's diffs to the calling surface, returning `(Vec<FileDiff>, DiffSessionType)` and leaving `Claimed(source)` behind. Returns `None` if the action is not prepared or was already claimed, so racing claim attempts are harmless.
- `discard_pending(action_id)`: drops per-action state. Called from the action model's terminal-result choke point (`BlocklistAIActionModel::handle_action_result` -> `BlocklistAIActionExecutor::discard_action_state`, `app/src/ai/blocklist/action_model.rs` / `execute.rs`), which every outcome — success, failure, cancellation, rejection — funnels through, so prepared content never outlives its action.
- `should_autoexecute`: allow continue-on-failure via `matches!(.., Some(PendingFileEdits::Failed(_)))`.
- `execute`: `match self.pending_file_edits.remove(id)`:
  - `Claimed(source)` -> `source.take_edits(ctx)` snapshots the surface's final state as `ClaimedEdits`. If the surface is gone (weak handle dead), returns `DiffApplicationFailed` with a recoverable message.
  - `Unclaimed` -> execution won the race against the `DiffsPrepared` subscriber (e.g. autoexecution, or headless with no registered surface). Wraps the still-owned diffs as `ClaimedEdits` with `final_content: None` per edit — persisting the unreviewed deltas is correct because nobody was reviewing.
  - `Failed` -> `DiffApplicationFailed`.
  The prepared/claimed arms call `PersistDiffModel::handle(ctx)` -> `resolve_and_persist(claimed, ..)`, returning `ActionExecution::new_async`. No `CodeDiffView` dependency, no executor-held completion channel (the persist model awaits its own writes internally), no per-caller resolution logic.
### GUI reroute (`block.rs`)
- `AIBlock` claims prepared diffs for its `CodeDiffView` via `claim_edits_for_view`: it boxes a `CodeDiffViewEditsSource(view.downgrade())` and calls `executor.claim_prepared_edits(action_id, source)`, feeding the returned diffs to the view (`set_diff_session_type` + `set_candidate_diffs`). `handle_requested_edit_complete` claims immediately if the diffs are already prepared, and a single block-level subscription to the executor (registered once in `AIBlock::new`) handles late `DiffsPrepared` events by looking the view up in the existing `requested_edits` map — no per-action subscriptions that accumulate over the block's lifetime. Claiming is idempotent from both trigger points because `claim_prepared_edits` only succeeds once.
- `CodeDiffViewEditsSource` (`code_diff_view.rs`) implements `PendingEditsSource` over a `WeakViewHandle<CodeDiffView>`, so the executor never keeps a dead review view alive. Its `take_edits` calls `CodeDiffView::claimed_edits`, which snapshots each file's `FileDiff` (path, base content from `InlineDiffView::diff_base_content`, op) paired with the editor buffer's final content (`None` for deletes — persistence derives the deletion from the op).
- On `CodeDiffViewEvent::TryAccept`: the view emits malformed-line telemetry (`send_malformed_line_telemetry`), then the block calls `action_model.execute_action` as today. No content is pushed — the executor pulls the reviewed state through the claimed source at execute time.
- Save-error surfacing: the failure now returns in `RequestFileEditsResult::DiffApplicationFailed`. `CodeDiffView`'s existing `FinishedAction` subscription observes a failed result for an `Accepted` diff and shows the save-failure toast (preserving the pre-refactor per-file `FailedToSave` toast behavior; the state stays `Accepted`, matching the old optimistic accept).
### Strip view-save machinery
Remove from `CodeDiffView`: `accept_and_save`, `SavingDiffs`, `SaveStatus`, `DiffApplicationState`, `accepted_file_diff_computed`, `handle_save_completed`, the result-assembly half of `try_emit_diffs_saved`, and the `SavedAcceptedDiffs` event; collapse `CodeDiffState::Accepted(Option<SavingDiffs>)` to a payloadless `Accepted` and fix its match sites (`is_complete`, revert guard, `try_accept`). Remove from `InlineDiffView`: `accept_and_save_diff` and `save_content`.
Keep GUI-side: editor rendering, delta application for display, `was_edited` tracking, the malformed-line/edited telemetry (relocated out of `try_emit_diffs_saved`, still computed from editor state at accept and emitted GUI-side), and post-accept revert (`restore_diff_base` + its `FileModel` registration).
### Passive-suggestion reroute (`terminal/view.rs`)
`on_maa_code_diff_generated` previously self-drove `view.accept_and_save` + `SavedAcceptedDiffs`. It now routes through the shared singleton: on `TryAccept`, snapshot `view.claimed_edits()` (the same execute-time payload the requested path produces) and hand it to `PersistDiffModel::handle(ctx)` -> `resolve_and_persist(claimed, ..)`. Passive diffs are not executor actions, so the view is the sole diff owner throughout and no claim is involved. The completion callback inspects the result and shows the save-failure toast on `DiffApplicationFailed` (the result is not surfaced to the LLM on this path). The existing `ContinuePassiveCodeDiffWithAgent` result reporting is unchanged.
### TUI surface (`crates/warp_tui`)
The TUI renders `RequestFileEdits` with a stateful per-action child view instead of a pure element:
- `TuiAIBlock` (`crates/warp_tui/src/agent_block.rs`) keeps `action_views: HashMap<AIAgentActionId, TuiToolCallView>`; `sync_action_views` (run at construction and on every output update) creates a `TuiFileEditsView` for each file-edit action that lacks one. `TuiToolCallView` exists because rendering only sees `&AppContext` and cannot create views; only tool types needing owned state get a variant.
- `TuiFileEditsView` (`crates/warp_tui/src/tui_file_edits_view.rs`) claims the prepared diffs (`try_claim` at construction plus a `DiffsPrepared` subscription for late preprocess completion) and owns them, satisfying the single-owner invariant on the TUI exactly as the GUI view does. The executor holds a `TuiFileEditsSource` (weak handle) as its `PendingEditsSource`.
- The TUI has no editable buffers, so `take_edits` returns each edit with `final_content: None` and persistence applies the diffs' deltas. The view renders a compact summary (`Edited N files (+a −r)`) from `FileDiff::line_stats`, or `Preparing edits…` before the claim succeeds.
### Relocate diff data types
Move `FileDiff`, `DiffBase`, and `DiffSessionType` out of `code_diff_view.rs` into `app/src/ai/blocklist/diff_types.rs` (plus the shared changed-lines helpers) so neither the executor nor the persist model imports a GUI view file. `diff_types.rs` also defines the executor/surface handoff types `ClaimedEdit { diff: FileDiff, final_content: Option<String> }` and `ClaimedEdits { edits, session_type }`, and `FileDiff::line_stats` for surface-side summaries. All consumers (`code_diff_view.rs`, `inline_diff.rs`, `request_file_edits.rs`, `block.rs`, `terminal/view.rs`, `passive_suggestions/maa.rs`, `warp_tui` via `tui_export.rs`) import directly from `diff_types` — no re-export shim.
### Call flow
Base (GUI-only), unchanged on the parent branch:
```
PHASE 1 preprocess: apply_diffs -> on_diffs_applied -> diff_view.set_candidate_diffs
                                                       (diffs live inside the view)
        |--------- GAP: user reviews / edits / clicks Accept ---------|
PHASE 2 execute: diff_views.get(id) -> accept_and_save
                     -> reads editor buffers -> FileModel::save -> result (view-produced)
```
New (surface-agnostic):
```
PHASE 1 preprocess: apply_diffs -> on_diffs_applied -> pending[id] = Unclaimed{diffs, session_type}
                                                       -> emits DiffsPrepared
        |-- a review surface claims: pending[id] = Claimed(source); the surface --|
        |   now solely owns the diffs (GUI editor buffers / TUI view state)       |
        |   GUI: user reviews/edits in place. TUI: summary row, no editing.       |
PHASE 2 execute: pending.remove(id)
                   Failed    -> DiffApplicationFailed
                   Claimed   -> source.take_edits() -> ClaimedEdits
                                (per-edit final_content from the surface; dead
                                 surface -> DiffApplicationFailed)
                   Unclaimed -> autoexec/headless won the claim race ->
                                ClaimedEdits with final_content: None
                 -> PersistDiffModel::resolve_and_persist(claimed)
                      final = final_content | base+deltas, per-file PersistAction
                 -> FileModel -> result
```
The result diff sent to the LLM becomes uniformly `similar`-based for both surfaces (previously the GUI used the editor-computed diff).
## Boundaries
- Prepared file content has exactly one owner at any time; the review surface's state is the only resident copy of file content while under review. The executor holds diff data only in the pre-claim `Unclaimed` window.
- Do not change the resolve side (`ApplyDiffModel` keeps its name and behavior).
- Revert stays GUI-local via `InlineDiffView::restore_diff_base`; the persist model does not own revert.
- No TUI review/approval UI: `TuiFileEditsView` is display-only, supplies no `final_content`, and persistence runs the delta-application branch.
## Testing and validation
Unit-test `PersistDiffModel::resolve_and_persist` (async `App::test`, await the future — see `app/src/ai/blocklist/persist_diff_model_tests.rs`):
- Create, update, delete each write via `FileModel` and return `RequestFileEditsResult::Success`.
- Update-with-rename routes through `rename_and_save` and reports the old path in `deleted_files`.
- Save failure returns `DiffApplicationFailed`.
- Per-edit `final_content` overrides delta-applied content; its absence falls back to delta application (headless path).
- `PersistAction::resolve` renames only on local sessions (remote and rename-to-same-path are plain writes), and the remote-rename outcome reports the update at the original path with nothing deleted — matching the actual write.
Unit-test the executor's claim lifecycle (`app/src/ai/blocklist/action_model/execute/request_file_edits_tests.rs`):
- `claim_prepared_edits` transfers ownership exactly once (second claim returns `None`).
- `execute` pulls claimed edits from the source; a vanished source fails recoverably with `DiffApplicationFailed`.
- `execute` on an unclaimed entry falls back to the diffs' deltas (`final_content: None`).
- `discard_pending` drops state in any state (unclaimed, claimed, failed).
Targeted runs then format + clippy:
```bash
cargo nextest run -p warp persist_diff_model request_file_edits
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
```
## Parallelization
Do not use child agents. The change is tightly coupled across the executor, one new submodel, the GUI block wiring, the code-diff view, and the passive-suggestion path in a single crate; splitting across worktrees would create more merge overhead than wall-clock savings.
Implementation sequence:
```mermaid
flowchart LR
  A["Relocate diff data types"] --> B["PersistDiffModel"]
  B --> C["Executor surface-agnostic rewrite"]
  C --> D["GUI block reroute"]
  D --> E["Strip view-save machinery"]
  E --> F["Passive-suggestion reroute"]
  F --> G["Tests + format + clippy"]
```
