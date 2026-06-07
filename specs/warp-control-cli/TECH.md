# Context
`PRODUCT.md` defines the launch contract for `warpctrl`: an allowlisted local control CLI usable only from a verified live Warp-managed terminal session and enabled by default. Every action requires a short-lived exact-action transport credential. Non-destructive actions that require no logged-in Warp-user authority, return no potentially sensitive user data, and modify only reversible UI/reviewed configuration state or return minimized non-sensitive structural metadata are default-authorized. Potentially destructive actions also require a non-scriptable one-shot confirmation in Warp.

`SECURITY.md` is the normative security architecture. The implementation must not add outside-Warp credential issuance, authenticated Warp-user grants, terminal block-content or history reads, command execution, direct Warp Drive operations, or any action absent from the product catalog.

The auth/security branch owns the complete retained catalog and security model. It must fold in useful handler work from higher branches rather than treating those branches as launch dependencies.

The existing app still provides useful implementation building blocks:
- The Warp binary already supports early command-mode dispatch, allowing the wrapper to avoid GUI startup.
- WarpUI's `ModelSpawner` provides background-to-main-thread execution for app-state reads and mutations.
- Existing window, workspace, pane-group, session, settings, theme, surface, and URI-intent actions provide user-visible behaviors that handlers should reuse.
- The existing typed local-control catalog, request envelope, response envelope, target types, and exact-action credential map provide a foundation that should be simplified to the new policy.

## Required invariants
- A caller cannot obtain any control credential without a valid proof from a live terminal session managed by the target Warp app instance.
- Warp control defaults to enabled, with a protected app-controlled setting retained as an emergency off switch.
- A proof is valid only for the instance and terminal session that issued it.
- No outside-Warp mode, external client context, external credential broker, or externally actionable discovery record ships.
- No action receives authenticated Warp-user authority.
- Every credential grants one exact action and expires quickly.
- Default-authorized actions satisfy the safe-action criteria: non-destructive, reversible UI/reviewed configuration mutation or minimized non-sensitive structural read, no logged-in Warp-user authority, and no potentially sensitive user-data result.
- Confirmation-required actions cannot execute without a one-shot in-app user gesture bound to the exact request.
- The bridge enforces enablement, proof/grant state, exact action, confirmation, parameters, and target restrictions locally.
- The bridge dispatches only typed allowlisted actions.
- Structural results never include excluded terminal contents, history, input buffers, environment values, paths, Drive data, or AI content.
- Removed actions fail as `not_allowlisted`; they are not retained as hidden stubs.

## Proposed changes
### 1. Simplify the protocol catalog
The catalog remains the source of truth for public action metadata. Each retained action declares:
- stable `ActionKind` and public name;
- implementation status;
- `requires_user_confirmation: bool`;
- target scope;
- typed parameter contract;
- typed result contract.

Remove catalog and protocol concepts that exist only for the discarded model:
- `InvocationContext` and outside/inside context lists;
- `ExecutionContextProof::ExternalClient`;
- `requires_authenticated_user`;
- `AuthenticatedUserRequirement`;
- authenticated-user subject and grant metadata;
- removed `ActionKind` variants and their parameter/result variants when no retained action uses them.

A verified terminal proof is no longer one invocation-context option among several; it is a prerequisite for using the protocol at all.

The retained catalog contains 76 actions:
- 73 actions with `requires_user_confirmation: false`;
- 3 actions with `requires_user_confirmation: true`.

The exact catalog is normative in `PRODUCT.md`. In particular, `block.list` remains only as content-free structural metadata. `block.inspect`, `block.output`, `history.list`, direct Drive actions, authenticated-user actions, and execution actions are removed. `file.list` is removed with the other potentially sensitive data reads. Every retained action must be implemented in the current branch.

### 2. Verified terminal proof lifecycle
When Warp creates a supported local terminal session, the app creates high-entropy proof material and records verifier state in an app-owned registry. The proof is bound to:
- the issuing app instance;
- terminal/session ID;
- process generation;
- issuance and expiry;
- revocation state.

The shell receives only the material needed for `warpctrl` to present the proof to its issuing app, such as an opaque instance-bound broker reference plus a short-lived proof secret or challenge-response input. Environment variables may carry the reference and proof material, but a caller-created label such as `INSIDE_WARP=1` is never sufficient.

The broker verifies the proof against the live registry before considering the requested action. It rejects proofs when the terminal closes, the app restarts, the proof expires, scripting is disabled, or the proof does not belong to the selected app instance.

Proof material may be inherited by descendants launched from the terminal. The proof therefore establishes that a request came through a live Warp-terminal execution context; it does not prove a human personally requested the action. Exact-action grants and required confirmation remain necessary.

### 3. Instance binding and discovery
The launch contract does not expose a globally discoverable control plane to external clients.

Recommended design:
- The proof-issuing terminal receives an opaque reference for contacting only its issuing app's credential broker.
- The reference and proof are not published in a global actionable discovery registry.
- The broker returns the endpoint or transport handle needed for a short-lived request only after proof and policy checks succeed.
- A credential and request are accepted only by the issuing app instance.
- `instance.list` returns the authorized proof-issuing instance. It does not enumerate or grant access to unrelated running Warp processes.

A per-process loopback listener may remain as the typed action transport if it preserves browser and localhost hardening, but knowing or guessing its port provides no authority. A future direct local IPC transport may replace loopback without changing action semantics.

### 4. Global enablement
Settings > Scripting exposes one local-only, app-controlled toggle: **Warp control scripting**.

Requirements:
- Default to enabled.
- Store the authoritative value in protected local state.
- Keep it out of Settings Sync, Warp Drive, server-backed preferences, ordinary user-editable settings, and the `warpctrl` settings allowlist.
- Allow changes only from Warp's Settings UI.
- On disable, revoke terminal proofs, pending confirmations, and active credentials.
- Do not publish any external mode or “enabled everywhere” option.

### 5. Exact-action credential issuance
After validating global enablement and terminal proof, the broker evaluates the requested exact action. For a default-authorized action, it automatically mints a short-lived credential after proof and action-policy validation. The credential is bound to:
- issuing instance;
- terminal session/proof;
- one exact `ActionKind`;
- optional target restrictions;
- issuance and expiry;
- unique revocation/audit identity.

The bridge revalidates current enablement, proof/session liveness, credential expiry, exact action, and target restrictions for every request. A credential for one action cannot invoke another action.

### 6. One-shot confirmation protocol
For `requires_user_confirmation: true`, proof and exact-action checks are necessary but insufficient. The action follows a confirmation flow:
1. Verify global enablement and terminal proof.
2. Validate the exact action and parameter shape.
3. Resolve only enough target state to identify the exact target safely.
4. Create a pending confirmation bound to the action, resolved target, relevant parameter digest, requesting terminal session, and short expiry.
5. Present an in-app prompt that clearly describes the requested effect.
6. Require a direct user gesture in Warp.
7. On approval, mint or activate a one-shot confirmation grant for that exact request.
8. Revalidate target identity and parameters, consume the grant, and execute.

Confirmation cannot be supplied through the wire protocol, CLI flags, stdin, environment variables, config, Agent Profile policy, an “always allow” choice, or another scripted action. Dismissal, denial, expiry, parameter changes, and target changes invalidate the request.

The confirmation prompt should display safe identifying information. It must not reveal excluded terminal content or secrets merely to ask for approval.

### 7. App-side request bridge
The control transport runs off the main UI thread, while WarpUI state belongs to the main app thread. Continue to use a `ModelSpawner<LocalControlBridge>` or equivalent main-thread bridge.

Request flow:
1. Authenticate the transport and locate the presented short-lived credential.
2. Decode the typed request envelope.
3. On the main thread, recheck scripting enablement, proof/session liveness, exact granted action, confirmation requirement/grant, target restrictions, and parameters.
4. Resolve deterministic targets.
5. Dispatch only the matching typed allowlisted handler.
6. Return a typed success or structured error response.

No handler may acquire broader authority from the CLI frontend or infer permission from a related action.

### 8. Target resolution
All requests are scoped to the proof-issuing app instance. Target resolution is reusable and deterministic:
1. Use the proof-bound instance.
2. Resolve window within the instance.
3. Resolve tab within the window.
4. Resolve pane within the tab or pane-group context.
5. Resolve session within the pane.
6. Resolve a block only for content-free `block.list`.
7. Resolve file paths and surfaces only for their declared actions.

Explicit opaque IDs resolve exactly or return `stale_target`. Active/default selection is allowed only when unambiguous. Index, title, name, and path helpers must fail on zero or multiple matches. Confirmation-required actions must revalidate the resolved target after approval so approval cannot be redirected by a race.

### 9. Result and parameter minimization
Structural metadata handlers may return only reviewed fields needed for targeting and UI orchestration. They must not return terminal commands, block output, input-buffer contents, history, environment values, working directories, open file paths, AI content, or Drive content.

`block.list` has a dedicated minimized result contract containing only opaque IDs, indices, status, timing, exit status, and similarly reviewed non-content metadata. Do not reuse an internal block model whose serialization contains command text or output.

`input.insert` and `input.replace` are default-authorized reversible UI changes and accept plain text only. Validation rejects newlines, carriage returns, Enter-equivalent input, control sequences, and submission behavior before dispatch.

Settings reads and writes use explicit reviewed allowlists. Default-authorized settings writes are limited to reversible local configuration and cannot include scripting enablement, auth/account/team state, security settings, private/internal settings, cloud-backed settings, irreversible settings, or settings that widen another control surface.

Surface actions return acknowledgement and target metadata only. They never serialize the underlying content of Warp Drive, AI assistant, code review, or another visible surface.

### 10. Handler families
Implement only handlers represented in the retained catalog:
- proof-bound instance/app metadata;
- safe action/capability metadata;
- structural window/tab/pane/session and content-free block metadata;
- reviewed app-state and metadata mutations;
- themes, appearance, reviewed local settings, and keybinding metadata;
- UI surface open/toggle actions;
- app-state-only file intents;
- default-authorized reversible input staging, session recovery, and reviewed settings writes;
- confirmed destructive close actions.

Do not implement handlers for terminal block content, history, input reads/clearing/mode changes/execution, authenticated-user flows, direct Drive objects, workflow execution, or command execution.

### 11. Current-branch delivery
The current branch must deliver the complete retained catalog and security boundary:
- Protected Warp control scripting toggle defaults to enabled and can revoke all authority when disabled.
- A live Warp-managed terminal receives an instance/session-bound proof.
- A caller without valid proof cannot obtain credentials or target state.
- The request reaches the proof-issuing app without external actionable discovery.
- The app bridge revalidates enablement, proof, and exact action before every dispatch.
- All 73 default-authorized actions execute without prompting after proof and policy checks.
- All 3 confirmation-required actions use the non-scriptable exact-request confirmation flow.
- All retained actions, generated surfaces, and structured success/failure responses are implemented and tested in this branch.

## Protocol errors
Retain or add stable errors for:
- `local_control_disabled`
- `execution_context_not_allowed`
- `invalid_terminal_proof`
- `unauthorized_local_client`
- `insufficient_permissions`
- `user_confirmation_required`
- `user_confirmation_denied`
- `user_confirmation_expired`
- `ambiguous_target`
- `missing_target`
- `stale_target`
- `invalid_selector`
- `invalid_request`
- `invalid_params`
- `unsupported_action`
- `not_allowlisted`
- `target_state_conflict`

Remove authenticated-user-specific errors once no protocol path can produce them.

## Testing and validation
Required coverage includes:
- explicitly disabled mode rejects proof and credential requests;
- fake, expired, revoked, wrong-session, and wrong-instance proofs fail;
- a caller without proof cannot discover actionable endpoints or app state;
- exact-action credentials cannot invoke a different action;
- credentials fail after terminal teardown, app restart, setting disable, or expiry;
- confirmation-required actions cannot execute through flags, protocol fields, stdin, configuration, profile policy, or reused approval;
- confirmation denial, dismissal, expiry, target changes, and parameter changes fail safely;
- confirmed requests execute at most once;
- removed actions decode as `not_allowlisted` or are rejected before dispatch;
- `block.list` never returns command text, output, working directory, or environment data;
- all other structural results exclude user content;
- input staging rejects submission and control characters;
- settings allowlists exclude security and authenticated/cloud-backed state;
- ambiguous, missing, and stale targets return structured errors;
- browser-origin and unauthenticated direct transport requests fail;
- credentials, proofs, terminal content, and sensitive parameters are absent from logs and errors.

Run protocol/catalog unit tests, bridge tests, proof lifecycle tests, confirmation-flow tests, parser/help snapshot tests, generated documentation checks, and the relevant workspace build/lint commands for the integrated current branch.

## Consolidation into the current branch
The current branch must absorb all launch-required implementation:
- retain verified terminal proof issuance and remove external invocation context, actionable external discovery, and external broker paths;
- retain the removal of authenticated-user grants, subject matching, and auth-specific errors;
- update `requires_user_confirmation` metadata and implement the confirmation broker/bridge flow for only the 3 confirmation-required close actions;
- remove the 22 excluded action variants, parser routes, handlers, tests, and docs;
- minimize `block.list` to structural metadata;
- fold in every retained read, target, UI, configuration, surface, and mutation handler from higher branches;
- exclude upstack Drive-data, execution, terminal-content, and history implementations;
- update generated help, completions, capability metadata, and built-in skill content;
- validate the complete retained catalog on the current branch.

Higher branches are not launch dependencies and may be dropped once useful retained-action work has been folded into the current branch.

## Follow-ups requiring separate review
Outside-Warp control, remote control, authenticated Warp-user actions, direct Drive operations, terminal-content reads, history reads, input execution, workflow execution, and agent-prompt submission each require separate future product/security design and an explicit catalog change. None is implicitly enabled by the launch architecture.
