# Core TUI Agent Model and Verifiable Send/Follow-Up Flow

## Problem statement
The branch has a `tui` WarpUI backend and a skeletal `warp-tui` binary, but the TUI app **cannot yet send prompts to Warp's agent backend or maintain a conversation**.

This plan adds the shared state and initialization needed for a TUI-native agent session that can:
1. Send an initial prompt.
2. Receive streamed output into conversation state.
3. Send a follow-up in the same conversation.

This is a minimal working slice, but it is built to extend. We prefer good seams now over throwaway stubs.

## Ultimate verifiable goal
A **headless, view-tree-free** test drives the TUI agent core. It:
- Initializes the TUI singleton graph in a test app context.
- Constructs a synthetic `AgentSessionOwnerId` from a fresh `EntityId` — no `TerminalView`, no `RootTuiView`, no runtime/driver. This proves conversation ownership is fully decoupled from views.
- Submits prompt `"first"` and feeds a fake streamed response.
- Submits follow-up prompt `"second"` and feeds another fake response.
- Asserts **one conversation with two ordered exchanges**, and that the second request carries the first request's conversation/task context.

**Target command (name may change):**
```sh
cargo test -p warp --features tui core_tui_model_sends_initial_prompt_and_follow_up
```
The test must verify both *request construction* and *history mutation* for the initial and follow-up prompts.

## Current state
### TUI entry path
- `app/src/bin/tui.rs:9` sets channel state and calls `warp::run_tui()`.
- `LaunchMode::Tui` exists at `app/src/lib.rs:400`.
- `run_internal` short-circuits **before** `initialize_app`, calling `crate::tui::init(ctx)` directly at `app/src/lib.rs:1078`. Enough for the logo-only TUI, not for agent requests.
- The TUI root at `app/src/tui.rs:45` is a view-only placeholder (logo/version) that starts `spawn_tui_driver` (`app/src/tui.rs:107`). It registers no agent state.

### GUI Agent Mode ownership
- `TerminalView` stores the per-pane Agent Mode cluster (`BlocklistAIController`, `BlocklistAIContextModel`, `BlocklistAIActionModel`, `BlocklistAIInputModel`) at `app/src/terminal/view.rs (2700-2720)` and constructs it at `app/src/terminal/view.rs (3451-3496)`.
- This is the stateful analog for the TUI work, but the TUI equivalent should be a **singleton model, not a view**.

### Reusable agent infrastructure
- `BlocklistAIHistoryModel` (`app/src/ai/blocklist/history_model.rs:208`) is the durable conversation store. Its terminology is terminal-specific: it keys live/cleared/active conversations by `terminal_view_id` (`app/src/ai/blocklist/history_model.rs (208-245)`). Conceptually these IDs are **local agent-session owners**, not necessarily terminal views.
- `ResponseStream` (`app/src/ai/blocklist/controller/response_stream.rs:76`) already wraps `generate_multi_agent_output`, cancellation, retry, stream-truncation detection, and stream events. **Good primitive to reuse.**
- Request construction flows through `RequestInput` and `api::RequestParams::new` (`app/src/ai/blocklist/controller.rs (2088-2507)`, `app/src/ai/agent/api.rs (92-345)`). `RequestParams::new` depends on shared singletons: settings, permissions, LLM preferences, execution profiles, server API, workspace state, MCP managers, API key managers, and network status.
- **Streamed agent text arrives via `ClientActions`**, not a separate channel. `apply_client_actions` (`app/src/ai/blocklist/history_model.rs:1716`) dispatches to `conversation.apply_client_action` (`app/src/ai/agent/conversation.rs:2367`), where `AddMessagesToTask` / `AppendToMessageContent` carry `AgentOutput` messages. Tool *execution* is queued separately from the finished output; with no tools advertised, none arrive.

## Proposed design

### 1. Introduce `AgentSessionOwnerId` (derived from the owning view)
A newtype over the owning **view's** `EntityId`. We key conversation state on the view, not on `CoreTuiModel`, so future non-root sub-views each get their own session and history with no extra plumbing.
```rust
/// Identifies the local owner of agent conversation state.
///
/// Backed by the `EntityId` of the owning TUI view (the root view today; any
/// sub-views later). `BlocklistAIHistoryModel` and `CoreTuiModel` key all
/// per-session state on this, replacing the current `terminal_view_id`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct AgentSessionOwnerId(EntityId);

impl AgentSessionOwnerId {
    pub fn new(view_id: EntityId) -> Self {
        Self(view_id)
    }

    pub fn entity_id(self) -> EntityId {
        self.0
    }
}
```
GUI `TerminalView` derives one from its view id; each TUI view derives one from its view id. The semantic owner becomes "the local agent session," decoupled from any specific view type.

### 2. Refactor `BlocklistAIHistoryModel` to own by `AgentSessionOwnerId`
A behavior-preserving rename from terminal-view ownership to agent-session ownership. Examples:
- `live_conversation_ids_for_terminal_view` → `live_conversation_ids_by_owner`
- `active_conversation_for_terminal_view` → `active_conversation_by_owner`
- `terminal_view_id_for_conversation` → `owner_id_for_conversation`
- `BlocklistAIHistoryEvent::terminal_view_id()` → `owner_id()`

GUI terminal call sites keep working by passing an `AgentSessionOwnerId::new(terminal_view_id)`. This is a sizable but mechanical change; it should land as its own PR ahead of the TUI model.

### 3. Split app initialization into phases
Factor `initialize_app` (`app/src/lib.rs:1142`) into shared and surface-specific phases so the TUI gets a **real app singleton graph**, not a minimal custom bootstrap.
- Shared phase: settings/auth/server/persistence and the AI singletons (`BlocklistAIHistoryModel`, `BlocklistAIPermissions`, `LLMPreferences`, `AIExecutionProfilesModel`, `AISettings`, `NetworkStatus`, MCP/API-key managers, `AIDocumentModel`, `AIRequestUsageModel`, etc.).
- GUI phase: workspace/pane/terminal/window models + GUI launch.
- TUI phase: `CoreTuiModel`, the root TUI window/view, and the TUI session/driver.

`LaunchMode::Tui` runs shared + TUI phases. The early `LaunchMode::Tui` short-circuit (`app/src/lib.rs:1078`) moves **after** shared initialization.

### 4. Add the `CoreTuiModel` singleton
The TUI app's stateful agent-session owner — the closest analog to the Agent Mode state `TerminalView` holds today, but as one app-level singleton keyed per session rather than per pane. Conversation/transcript content stays in the shared `BlocklistAIHistoryModel`; `CoreTuiModel` owns only the send pipeline and per-session pointers into history.
```rust
/// App-level singleton owning agent send/receive state for the TUI app.
///
/// Holds one `TuiAgentSession` per owning view (`AgentSessionOwnerId`), so multiple
/// concurrent sessions work out of the box even though the root view is the
/// only owner today.
pub struct CoreTuiModel {
    sessions: HashMap<AgentSessionOwnerId, TuiAgentSession>,
}

impl Entity for CoreTuiModel {
    type Event = CoreTuiModelEvent;
}

impl SingletonEntity for CoreTuiModel {}

/// Per-view agent state.
struct TuiAgentSession {
    /// Conversation the next prompt follows up in / the last one streamed to.
    /// `None` until the first prompt starts a conversation.
    active_conversation_id: Option<AIConversationId>,
    /// The in-flight request for this session, if a response is streaming.
    in_flight: Option<InFlightRequest>,
}

/// Tracks a streaming request. The `ResponseStream` handle is held for the
/// duration of the stream and dropped on completion/cancel.
struct InFlightRequest {
    conversation_id: AIConversationId,
    stream_id: ResponseStreamId,
    response_stream: ModelHandle<ResponseStream>,
}
```
Public API:
```rust
impl CoreTuiModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self;

    /// Idempotently registers a session for `owner`.
    pub fn register_session(
        &mut self,
        owner: AgentSessionOwnerId,
        ctx: &mut ModelContext<Self>,
    );

    /// Sends `prompt` for the session: starts a new conversation if the session
    /// has none, otherwise follows up in its active conversation. Errors if a
    /// request is already in flight for that session.
    pub fn send_prompt(
        &mut self,
        owner: AgentSessionOwnerId,
        prompt: String,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<(AIConversationId, ResponseStreamId)>;

    /// Cancels the in-flight request for the session, if any.
    pub fn cancel_active_request(
        &mut self,
        owner: AgentSessionOwnerId,
        ctx: &mut ModelContext<Self>,
    );

    pub fn active_conversation_id(
        &self,
        owner: AgentSessionOwnerId,
    ) -> Option<AIConversationId>;

    pub fn has_in_flight_request(&self, owner: AgentSessionOwnerId) -> bool;
}
```
Events (so future transcript/input views observe instead of polling):
```rust
pub enum CoreTuiModelEvent {
    /// A prompt was accepted and a request was sent.
    PromptSubmitted { owner: AgentSessionOwnerId, conversation_id: AIConversationId },
    /// Streamed output mutated the conversation (drives transcript repaints).
    ConversationUpdated { owner: AgentSessionOwnerId, conversation_id: AIConversationId },
    /// The active request reached a terminal state (success or error).
    RequestFinished { owner: AgentSessionOwnerId, conversation_id: AIConversationId },
}
```
Shared singletons it reads (all via the phased initializer in §3): `BlocklistAIHistoryModel` (conversation state); `LLMPreferences` + `AIExecutionProfilesModel` (model ids / context window); `BlocklistAIPermissions` + `AISettings` (request settings); `ServerApiProvider` (transport, via `ResponseStream`); `NetworkStatus` (retry behavior). It creates `ResponseStream` models per request.

### 5. The minimal request path
`CoreTuiModel::send_prompt` mirrors the backend-neutral core of `BlocklistAIController::send_query` + `send_request_input`, minus terminal/UI coupling:
1. Resolve the session; reuse `active_conversation_id` or call `BlocklistAIHistoryModel::start_new_conversation(owner, …)`.
2. Build context via the TUI context builder (§7) and an `AIAgentInput::UserQuery`.
3. Build `RequestInput`, then `api::RequestParams::new(...)` with an empty `supported_tools_override` (§6).
4. Create a `ResponseStream`, record the `InFlightRequest`, and call `BlocklistAIHistoryModel::update_conversation_for_new_request_input` to append the user exchange; mark the conversation `InProgress`.
5. Subscribe to `ResponseStreamEvent` and fold events into history exactly as the GUI controller does:
    - `Init` → `initialize_output_for_response_stream` (captures the server conversation token).
    - `ClientActions` → `apply_client_actions`. **This is the channel for streamed agent text** (`AddMessagesToTask` / `AppendToMessageContent` carrying `AgentOutput`), not just tools — it must be handled even with tools disabled.
    - `Finished` → `mark_response_stream_completed_successfully` (or the error variant).
    - `AfterStreamFinished` → clear `in_flight`, emit `RequestFinished`.

The follow-up reuses the session's `active_conversation_id`, so step 3 sends the prior `ConversationData` (tasks + server token), giving the second request the first request's context.

### 6. No client tools in phase one
Pass `supported_tools_override: Some(vec![])` so the server advertises no client tools. This scopes the first milestone to **text exchange and follow-up continuity**, with no need for `BlocklistAIActionModel`.

This does *not* remove streamed text — agent output still arrives via `ClientActions` (§5). Because no tools are advertised, no executable client actions are expected. The action-execution + shell-interleaving design is deferred to a later PR, so `CoreTuiModel` should leave a clear seam for it (e.g. a future `actions` collaborator) rather than hard-coding "no actions" assumptions throughout the send/stream path.

### 7. A small TUI context builder
Rather than depending on `BlocklistAIContextModel`, add a focused builder that returns the `Arc<[AIAgentContext]>` and `SessionContext` a TUI query needs:
```rust
/// Builds the request context for a TUI agent query. Phase one includes only
/// session-independent context; richer context (attachments, selections,
/// project rules) can be layered in later without changing call sites.
pub struct TuiAgentContextBuilder;

impl TuiAgentContextBuilder {
    /// Directory + current time + execution environment context.
    pub fn context(app: &AppContext) -> Arc<[AIAgentContext]>;

    /// A local, non-terminal `SessionContext` (no terminal session/shell).
    pub fn session_context(app: &AppContext) -> SessionContext;
}
```
Phase-one context: current working directory + home, current time, and execution-environment (OS/shell) where available.

Explicitly excluded for now: terminal block selection, selected terminal text, pending attachments, long-running command snapshots.

`SessionContext`'s fields are private and it only has terminal/test constructors today, so this needs a small additive constructor (e.g. `SessionContext::local(cwd)`). That is preferable to widening the existing terminal constructor.

## Testing strategy

### Primary end-to-end model test (headless, no view tree)
Drives `CoreTuiModel` directly using a synthetic `AgentSessionOwnerId` from a fresh `EntityId` (no `TerminalView`, no `RootTuiView`, no runtime/driver), proving conversation ownership is fully decoupled from views.
- Must **not** hit the live Warp server. Emit a deterministic fake stream of `Init`, `ClientActions` (an `AddMessagesToTask` carrying agent text), and `Finished`.
- If `ResponseStream` is too coupled to `ServerApiProvider`, add a narrow transport seam / test-only constructor rather than special-casing `CoreTuiModel`.
- Asserts that `CoreTuiModel`:
    - Registers as a singleton and tracks state under the `AgentSessionOwnerId`.
    - Starts a new conversation on the first prompt and appends the user query + streamed agent text to `BlocklistAIHistoryModel`.
    - Marks the conversation successful after `Finished`.
    - Sends the second prompt as a **follow-up in the same conversation**, with the second `RequestParams` carrying the first request's task/server context.

### Refactor regression test
Cover `BlocklistAIHistoryModel` ownership after the `AgentSessionOwnerId` rename: starting a conversation, marking active, transferring ownership, and filtering events by agent session must match the old terminal-view semantics.

### Initializer coverage
Verify that `LaunchMode::Tui` runs the shared AI/server singleton bootstrap **before** registering `CoreTuiModel`. No need to launch the terminal UI — just prove the models required by request construction exist in a headless app context.

## Non-goals
- **No final UI yet:** no transcript rendering, scrollable transcript, text input editor, shell-command interleaving, or model selector. This plan prepares the state/request pipeline those PRs read and write.
- **No `BlocklistAIActionModel` / tool execution.** Pass empty supported tools. Leave a seam; don't build the action layer.
- **No wholesale rename** of every UI string or historical DB concept from terminal to agent session. The refactor targets code ownership semantics around `BlocklistAIHistoryModel` APIs and events.

## Sequencing
1. **History ownership refactor** — introduce `AgentSessionOwnerId`, rename `BlocklistAIHistoryModel` ownership APIs/events, update GUI call sites. Behavior-preserving. (Refactor regression test.)
2. **Initializer split** — phase `initialize_app`; route `LaunchMode::Tui` through shared + TUI phases. (Initializer coverage.)
3. **`CoreTuiModel` + context builder + request path** — text-only, no tools. (Primary end-to-end model test.)

Later PRs (out of scope): transcript view, input view, action/shell execution.

## Open decisions before implementation
- **`AgentSessionOwnerId`:** resolved — a newtype over the owning view's `EntityId`, defined near the agent conversation/history types (per-view, not per-model, to support future sub-views).
- **Initialization factoring boundary:** preferred direction is splitting `initialize_app` into shared, GUI-specific, and TUI-specific phases; exact cut points settled during the initializer PR.
- **`SessionContext` constructor:** needs a small additive local (non-terminal) constructor; confirm shape during implementation.
- **Response-stream test seam:** reuse `ResponseStream` if a fake stream source slots in cleanly; otherwise introduce a narrow request-driver/transport abstraction so `CoreTuiModel` is testable without network calls.
