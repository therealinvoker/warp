# warpctrl security architecture
`warpctrl` is a local control CLI for operating the visible state of a running Warp app from a verified live Warp-managed terminal session. The launch security model intentionally excludes outside-Warp invocation, authenticated Warp-user authority, terminal block-content reads, shell history reads, input execution, direct Warp Drive operations, and remote control.

The security architecture is a local app-enforced capability system designed to be enabled by default. A protected emergency setting can disable the feature, a live terminal proof establishes the only eligible execution context, the app issues one short-lived credential for one exact action, confirmation-required actions require a direct one-shot in-app gesture, and the app bridge validates all policy and targeting before typed dispatch.

Exact-action credentials and confirmation are safety and intent mechanisms. They are meaningful boundaries against browsers, other OS users, unauthenticated clients, external same-user processes without proof, stale instances, and accidental overreach by honest automation. They are not a claim that a fully compromised same-user process or inherited process inside a Warp terminal can be perfectly isolated.

## Launch policy
- Warp control scripting is enabled by default.
- Only a caller presenting a valid proof from a live Warp-managed terminal session may request credentials.
- A proof is bound to one app instance and terminal session.
- Every credential grants one exact action, expires quickly, and is valid only for its issuing instance and proof/session.
- Default-authorized actions are limited to non-destructive operations that require no logged-in Warp-user authority, return no potentially sensitive user data, and modify only reversible UI/reviewed configuration state or return minimized non-sensitive structural metadata.
- Actions marked `requires_user_confirmation: true` are potentially destructive and require one direct in-app approval bound to the exact action, target, parameters, and requesting session.
- There is no outside-Warp mode, external invocation context, external credential broker, or externally actionable discovery record.
- There are no authenticated Warp-user grants or actions.
- There are no public actions for terminal block contents, shell history, terminal input reads, command execution, direct Warp Drive data, or AI conversation contents.

## Security goals
- Prevent browsers, network peers, other OS users, unauthenticated clients, and external same-user processes without terminal proof from controlling Warp.
- Prevent merely discovering or guessing a local endpoint from granting authority.
- Preserve a protected emergency control that can disable Warp control immediately.
- Verify live Warp-terminal execution context without trusting a caller-declared label.
- Bind proof and credentials to the issuing app instance and terminal session.
- Enforce exact typed actions in the app bridge before dispatch.
- Require non-scriptable explicit confirmation for potentially destructive actions.
- Permit reversible UI and reviewed configuration changes by default, including persistent changes.
- Prevent confirmation from becoming a reusable or pre-authorized broad grant.
- Preserve deterministic targeting so approval or authority cannot silently shift to a different target.
- Keep structural result data free of terminal contents and other excluded user data.
- Keep the action surface finite, typed, documented, and excluded by default.
- Avoid logging credentials, proof material, sensitive parameters, or excluded content.

## Non-goals
- Protecting against kernel, administrator, hypervisor, or complete same-user compromise.
- Proving that a human initiated every default-authorized action.
- Treating a Warp-terminal proof as authenticated Warp-user identity.
- Supporting arbitrary same-user desktop automation outside Warp.
- Supporting remote control.
- Supporting authenticated cloud-backed data or operations.

## Trust boundaries
### Operating-system boundary
Private local state, sockets, and transports must be accessible only to the owning OS user. This protects against other users but does not distinguish among processes already running as that user.

### External-process boundary
Outside-Warp processes are not eligible clients. Warp does not publish an actionable global control record or issue credentials merely because a process has the same UID. A caller must possess and successfully present proof issued to a live Warp-managed terminal session.

### Warp-terminal proof boundary
A terminal proof establishes that a request flows through a live Warp-managed terminal session. The app verifies it against app-owned session registry state. A caller cannot gain this status by setting an environment variable or claiming a PID/session ID.

Proof material may be inherited by descendant processes launched from the terminal. The proof therefore raises eligibility for the limited public catalog but does not prove personal human intent. Potentially destructive actions require confirmation even with valid proof.

### Action boundary
Every credential grants one exact `ActionKind`. The bridge compares the request to the granted action and direct action policy before target resolution and dispatch. Similar effects do not share authority.

### Confirmation boundary
A valid confirmation is a one-shot capability for one exact request. It is bound to the action, resolved target, relevant parameter digest, requesting terminal session, and expiry. It cannot authorize a related action, changed parameters, a changed target, another terminal, or a later request.

### Target boundary
Proofs and credentials are bound to the issuing app instance. Optional narrower scopes may bind authority to a specific window, tab, pane, session, file intent, or surface. Targets must be resolved deterministically and revalidated after confirmation.

## Threat model
### In scope
- Browser-origin JavaScript attempting to call local control endpoints.
- Network peers and other OS users attempting to control a Warp instance.
- External same-user processes attempting to discover or request control authority without a live terminal proof.
- Callers forging, replaying, or widening terminal proofs, credentials, or confirmations.
- Valid inside-Warp callers attempting an action different from the exact action granted.
- Valid inside-Warp callers attempting to bypass confirmation.
- Confirmation reuse, parameter substitution, target substitution, or execution after expiry.
- Stale sessions, app restarts, and stale target IDs.
- Malformed, unsupported, removed, or unallowlisted actions.
- Structural metadata accidentally leaking excluded terminal or user content.
- Input staging accidentally submitting or executing text.

### Out of scope
- A malicious process with arbitrary same-user process/memory access that can steal live proof material or automate direct UI gestures.
- Kernel, administrator, hypervisor, or physical compromise.
- Remote transport security.

## Authoritative emergency control
Settings > Scripting exposes one app-controlled setting: **Warp control scripting: Disabled / Enabled**.

Requirements:
- Default to enabled.
- Store the authoritative value in protected local storage.
- Keep it local-only and absent from Settings Sync, Warp Drive, server-backed preferences, ordinary settings files, generated public settings schemas, and the `warpctrl` settings allowlist.
- Permit changes only through Warp's Settings UI.
- Do not provide CLI flags, protocol actions, config keys, defaults/registry writes, or server preferences that change it.
- On disable, revoke terminal proofs, pending confirmations, and active credentials immediately.

The protected setting is an emergency off switch, not the ordinary authorization boundary. The live terminal proof and action policy make default enablement safe; the setting provides an app-controlled way to revoke the feature immediately. It is not itself proof of user intent for each action.

## Terminal proof model
When Warp creates an eligible local terminal session, it creates high-entropy proof material and an app-owned verifier entry containing:
- issuing instance ID;
- terminal/session ID;
- app/process generation;
- issued-at and expiry;
- revocation state.

The terminal receives only the instance-bound reference and proof material needed to contact its issuing app. The app validates proof possession and registry state before issuing any credential.

Proof requirements:
- Caller-supplied labels, environment markers, PIDs, usernames, or session IDs are never sufficient alone.
- Proofs are invalid across app instances.
- Proofs are revoked on terminal teardown, app restart, setting disable, explicit revocation, or expiry.
- Proof verification occurs before credential issuance or disclosure of app state.
- Proofs and broker references are never printed in command output, errors, logs, help, completions, or generated docs.
- Proof material should be rotated or challenge-bound where practical to reduce replay.

Descendant processes may inherit proof material. Actions requiring stronger evidence of current human intent must use the confirmation model below.

## Discovery and transport
There is no externally actionable global discovery registry for launch. The proof-issuing terminal receives an opaque reference to its issuing app's broker. A process without that reference and valid proof cannot obtain an action endpoint or credential.

If the typed action transport uses loopback HTTP:
- Bind only to `127.0.0.1` on an instance-local port.
- Do not set permissive CORS headers.
- Reject requests carrying an `Origin` header.
- Require exact expected `Host` metadata.
- Require a valid short-lived credential before decoding or dispatching the typed action.
- Keep unauthenticated responses minimal and non-sensitive.

Knowing or guessing a loopback port is never sufficient. A future direct IPC transport may replace loopback, but it must preserve proof, exact-action, confirmation, and app-side validation semantics.

## Credential model
After validating enablement and terminal proof, the app broker may issue a short-lived exact-action credential. A credential encodes or references:
- issuing app instance;
- terminal session/proof identity;
- one granted `ActionKind`;
- optional target restrictions;
- issuance and expiry;
- unique credential ID for revocation and safe audit correlation;
- integrity protection.

Credentials must never contain or imply authenticated Warp-user authority. They must never be printed, logged, stored in discovery records, or reused for another action.

The bridge rechecks current enablement, proof/session liveness, credential validity, exact action, confirmation policy, and target restrictions on every request. Disabling scripting or closing the issuing terminal invalidates otherwise unexpired credentials.

## Explicit user confirmation
The following retained actions require one-shot confirmation:
- `window.close`
- `tab.close`
- `pane.close`

Confirmation requirements:
- Warp displays an in-app prompt after proof, action, and parameter validation and safe target resolution.
- The prompt identifies the action, exact target, and relevant effect or parameters without revealing excluded content or secrets.
- Approval requires a direct gesture in Warp.
- The approval is bound to the exact action, target, parameter digest, requesting terminal session, and short expiry.
- Approval is consumed at most once.
- Denial, dismissal, expiry, target changes, parameter changes, or terminal teardown invalidate the request.
- The bridge revalidates the target and parameters immediately before execution.

Confirmation cannot be supplied or bypassed by:
- a CLI flag such as `--yes` or `--force`;
- stdin or another shell process;
- environment variables;
- config files or ordinary preferences;
- Agent Profile policy;
- an “always allow” or remembered choice;
- another `warpctrl` request;
- a previously approved related action.

Repeated or abusive confirmation requests should be rate-limited or coalesced without weakening one-shot semantics.

## Action policy
The exact public catalog and confirmation labels are normative in `PRODUCT.md`. New actions are excluded by default and require explicit product/security review.

### Default-authorized actions
Default-authorized actions remain limited to non-destructive visible UI control, minimized non-sensitive structural metadata, reviewed reversible appearance/configuration behavior, safe surface toggles, reversible input staging, session recovery, and file-open intent. Persistent reversible changes are allowed. These actions still require global enablement, valid terminal proof, exact-action credentials, deterministic targeting, and bridge enforcement.

### Content-free block metadata
`block.list` is the only public block action. Its result may contain only opaque IDs, indices, status, timing, exit status, and similarly reviewed non-content metadata. It must not contain command text, output, working directories, environment values, prompts, or other block contents.

`block.inspect` and `block.output` are not allowlisted. Direct or internal requests for them return `not_allowlisted`.

### Terminal history and input
`history.list`, `input.get`, `input.clear`, `input.mode.set`, and `input.run` are not allowlisted.

`input.insert` and `input.replace` are default-authorized reversible UI changes. They may stage plain text only and must reject newline, carriage return, Enter-equivalent input, terminal control sequences, and submission or execution behavior. Accepted-command submission and agent-prompt submission remain excluded.

### Settings
Settings actions use explicit reviewed allowlists. Reads expose only non-secret, local, non-security-sensitive values. Default-authorized writes are limited to reversible local configuration and cannot change scripting enablement, authentication, account/team state, security settings, private/internal settings, cloud-backed settings, irreversible settings, or settings that widen another control surface.

### Files and surfaces
`file.open` is an app-state intent using a caller-supplied path and does not read file contents. Open-file listing, arbitrary filesystem traversal, and file-content operations are excluded.

Surface actions may show, focus, or toggle existing UI only. They return no underlying Drive, AI, code-review, or other surface content and cannot execute or mutate authenticated/cloud-backed data.

### Authenticated and cloud-backed operations
There is no authenticated scripting boundary in the launch protocol because no public action may require authenticated Warp-user authority. Auth status/login, all direct Warp Drive actions, workflow execution, authenticated settings/data, and similar operations are removed rather than gated behind an authenticated grant.

## Removed action families
The launch catalog explicitly removes:
- `auth.status` and `auth.login`;
- `block.inspect`, `block.output`, `history.list`, and `file.list`;
- `input.get`, `input.clear`, `input.mode.set`, and `input.run`;
- every direct `drive.*` action.

It also excludes AI conversation reads, environment-variable reads, local file-content operations, arbitrary settings, arbitrary internal dispatch, debug/token/crash actions, accepted-command submission, and agent-prompt submission.

Removed actions must not remain as hidden capabilities or grantable credential scopes. They are absent from the public action enum, parsers, generated help, completions, capability metadata, handlers, and operator docs.

## Target scoping and deterministic resolution
Targeting is part of authorization and confirmation.

Rules:
- Requests are fixed to the proof-issuing app instance.
- Explicit opaque IDs resolve exactly or return `stale_target`.
- Active/default selectors are allowed only when unambiguous.
- Index/title/name/path selectors must fail on zero or multiple matches.
- A request must never silently retarget after a stale or ambiguous selection.
- Confirmation-required actions bind approval to a resolved target and revalidate that target before execution.
- A credential scoped to one target cannot act on another target.

Target resolution must not disclose excluded content in errors or confirmation prompts.

## Result minimization
Default-authorized structural reads may expose only reviewed metadata necessary for targeting and UI orchestration. They must not expose:
- terminal command text or output;
- input-buffer contents;
- shell history;
- environment values;
- working directories or open file paths;
- AI conversation content;
- Warp Drive object data;
- secrets or credentials.

When a useful result would require excluded content, the action must be removed or redesigned rather than silently expanding an existing result contract.

## Browser and localhost protections
Loopback is not sufficient by itself. Any local transport must assume malicious webpages can guess ports and send blind requests.

Required protections:
- no permissive CORS;
- reject `Origin` headers;
- exact expected host/endpoint validation;
- no JSONP or browser-readable fallback;
- proof-gated credential issuance;
- valid exact-action credentials for typed requests;
- no secrets in browser-readable locations;
- minimal unauthenticated errors;
- no mutating GET endpoints.

## Auditing and logging
Safe audit records may contain:
- timestamp;
- issuing instance and terminal/session opaque IDs;
- credential or confirmation audit ID;
- exact action name;
- target type and opaque ID when safe;
- confirmation outcome for confirmation-required actions;
- success or structured error code.

Never log:
- terminal proof material;
- credentials or confirmation capabilities;
- terminal commands or output;
- input text;
- history;
- environment values;
- file paths;
- setting values when sensitive;
- Drive, AI, or other excluded content.

Normal denials, user confirmation rejections, and selector failures should not use error-level logs unless developer attention is required.

## Security errors
Important structured errors include:
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

Authenticated-user-specific errors should be removed because authenticated-user grants and actions do not exist in the launch model.

## Required controls before an action ships
Before any retained action is marked implemented:
- The action exists in the reviewed product catalog with the correct confirmation label.
- Global scripting state, terminal proof, exact-action credential, and bridge checks are enforced.
- The action has been reviewed as either default-authorized under the safe-action criteria or confirmation-required because it is potentially destructive.
- Removed or related actions cannot reuse its credential.
- Its parameters and result are minimized and typed.
- Its target resolution is deterministic.
- Confirmation-required actions enforce one-shot confirmation and cannot be scripted around.
- Default-authorized mutations are reversible and cannot execute input, widen authority, or require logged-in Warp-user authority.
- Tests cover valid proof, invalid proof, different-action credentials, stale/ambiguous targets, and action-specific restrictions.
- Structural results are tested against excluded-content leakage.
- Logs and errors contain no proof, credential, sensitive parameter, or excluded content.
- Generated help, completions, capability metadata, and docs reflect actual support.

## Platform requirements
On every platform, proof registries, broker transports, and credential state must be owner-only and instance-bound.

On macOS, protected enablement and any stored long-lived proof material should use Keychain or equivalent Warp-signed app-controlled storage. On Windows, use Credential Manager, DPAPI-backed protected storage, or equivalent app-controlled storage. On Linux, prefer the platform secret service, with any owner-only fallback documented as weaker.

Platform support is incomplete until proof issuance, revocation, protected enablement, transport restrictions, and tests exist for that platform. Do not fall back to outside-Warp or same-UID-only credential issuance on platforms without the proof path.

## Future capabilities require separate review
Outside-Warp control, remote control, authenticated Warp-user actions, direct Warp Drive operations, terminal block-content reads, history reads, input execution, workflow execution, and agent-prompt submission require separate product/security design and an explicit catalog change. They must not reuse the launch policy by default.
