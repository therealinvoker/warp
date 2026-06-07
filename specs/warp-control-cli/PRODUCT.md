# Summary
Warp should ship an allowlisted local control CLI command, provisionally named `warpctrl`, for operating the running Warp app from a verified Warp-managed terminal session. `warpctrl` is exposed as an Oz-style wrapper script that invokes the existing channel-specific Warp binary in control mode rather than as a separate standalone binary.

The launch contract is deliberately narrow enough for Warp control to be enabled by default for verified live Warp terminals. `warpctrl` does not accept outside-Warp invocations, does not act with authenticated Warp-user authority, does not expose Warp Drive objects, does not execute terminal input, and does not return terminal block contents or shell history. Every request uses a short-lived credential for one exact action. Non-destructive actions that only modify reversible UI or reviewed configuration state, or return minimized non-sensitive structural metadata, are authorized by default. Potentially destructive actions additionally require a one-shot explicit confirmation in Warp that cannot be scripted or pre-approved.

## Problem
Warp already has rich interactive actions, but most are reachable only through UI, keybindings, menus, or deeplinks. Agents and developers working inside Warp cannot reliably inspect or operate Warp's own product surfaces through a stable typed interface. Screen automation and arbitrary internal action dispatch are brittle and unsafe, while native agent tools do not cover arranging Warp windows, tabs, panes, sessions, and visible app surfaces.

## Goals / Non-goals
Goals:
- Provide a first-class `warpctrl` command for scripts and agents running from verified Warp-managed terminal sessions.
- Make Warp's visible local UI and structural app state available through a finite typed control plane.
- Enable Warp control by default while limiting default authority to actions safe enough for every verified live Warp terminal.
- Require one exact-action transport credential for every request without treating automatic issuance for a default-authorized action as a user permission grant.
- Require a non-scriptable, one-shot in-app confirmation for potentially destructive actions.
- Allow reversible UI and reviewed configuration changes by default, including persistent changes such as rename, tab color, theme, and appearance settings.
- Make targeting deterministic across the proof-issuing instance, windows, tabs, panes, sessions, and other allowlisted nouns.
- Keep startup lightweight by using the existing Warp binary's early control-mode dispatch.
- Implement the complete retained launch catalog in the current branch.

Non-goals:
- Allowing `warpctrl` invocations from external terminals, IDEs, launch agents, background services, cron jobs, or arbitrary same-user processes.
- Acting on behalf of a logged-in Warp user or exposing authenticated Warp data.
- Reading terminal block contents, terminal input buffers, shell history, environment variables, or AI conversation contents.
- Executing commands, submitting terminal input, submitting agent prompts, or running workflows.
- Reading or mutating Warp Drive objects.
- Replacing native tools for file content, shell execution, web/MCP calls, or attached agent context.
- Exposing arbitrary internal actions, arbitrary settings, debug helpers, or a general RPC escape hatch.
- Controlling a different Warp process than the one that issued the verified terminal proof.
- Remote control.

## Primary user stories
1. **Agent workspace orchestration.** An agent running inside Warp can inspect structural app state, create or reuse a local window/tab/pane layout, name and focus targets, and open visible Warp surfaces. It continues to use native tools for code edits, file reads, shell execution, MCP calls, and other non-UI work.
2. **Existing-session navigation and repair.** A user or agent can determine which window, tab, pane, or session is active and restore a useful visible layout without reading terminal commands or output through `warpctrl`.
3. **Deterministic demos and walkthroughs.** A script started inside Warp can put the proof-issuing app instance into a known presentation state using themes, zoom, windows, tabs, panes, focused targets, panels, and command surfaces.
4. **Personalization and onboarding.** An agent can inspect reviewed local settings, propose Warp equivalents, and apply reversible allowlisted changes without interrupting the user for each persistent preference update.
5. **Inside-Warp power-user scripting.** A developer can compose safe, typed UI operations from a Warp terminal while potentially destructive actions remain visibly user-confirmed.

## Launch security model
### Verified inside-Warp invocation only
`warpctrl` is usable only from a live Warp-managed terminal session that can present an app-issued proof. The proof is bound to the issuing app instance and terminal session. A caller without valid proof receives `execution_context_not_allowed` before it can obtain action credentials or app state.

There is no outside-Warp mode, external-client invocation context, external credential broker, or externally actionable discovery record. A valid proof authorizes requests only against the app instance that issued it. `instance.list` therefore reports the authorized proof-issuing instance rather than granting cross-process control.

A verified terminal proof establishes execution context, not personal user intent. Descendant processes may inherit access to proof material, so potentially destructive actions still require explicit confirmation.

### Global enablement
Settings > Scripting exposes one local-only app-controlled setting:
- **Warp control scripting: Disabled / Enabled**

The default is enabled so every verified live Warp terminal can use the default-authorized catalog without setup. The setting remains a protected emergency control. Only Warp's Settings UI may change it. `warpctrl`, direct protocol requests, shell scripts, ordinary preferences, synced settings, and settings actions cannot change it. Disabling the setting invalidates active credentials and prevents new requests.

### Exact-action credentials
Every request requires a short-lived credential for one exact typed action. Authority for one action never implies authority for another action. Credentials are bound to the proof-issuing instance and terminal session, and may include narrower target restrictions. For a default-authorized action, credential issuance is automatic after proof and policy validation and is not presented as a permission prompt.

### Explicit user confirmation
Actions marked `requires_user_confirmation: true` are potentially destructive. They require a direct one-shot gesture in Warp before execution or sensitive result release. Confirmation is bound to:
- the exact action;
- the resolved target;
- the relevant parameters or their safe digest;
- the requesting terminal session;
- a short expiry.

Confirmation cannot be satisfied by CLI flags, stdin, environment variables, config files, Agent Profile policy, an “always allow” option, or another control request. Approval is consumed by the exact request. Denial, dismissal, target changes, parameter changes, or expiry fail the request.

## Behavior
1. `warpctrl` operates only against the running Warp app instance that issued the caller's verified terminal proof.
2. The CLI exposes only the explicitly allowlisted actions below. Unknown, removed, unsupported, or malformed actions fail with structured errors and are never forwarded to arbitrary internal dispatch.
3. Every request checks global enablement, terminal proof, exact-action authority, direct action policy, and target restrictions before dispatch. Default-authorized actions proceed without a user prompt.
4. Confirmation-required actions do not execute and do not return sensitive results until Warp records a valid one-shot confirmation for that exact request.
5. Successful mutations identify the executing instance and resolved target when applicable.
6. Protocol and runtime failures contain stable machine-readable error codes, human-readable explanations, and safe selector details.
7. The CLI supports human-readable output by default and stable JSON output for scripts. Errors never include credentials or excluded sensitive data.
8. Explicit opaque target IDs must resolve exactly or return `stale_target`. Active, index, title, name, and path selectors must fail rather than silently choosing among ambiguous targets.
9. The protocol is command-oriented and versioned. Each action has a stable name, typed parameters, typed result, target scope, implementation status, and confirmation requirement.
10. New actions are excluded by default. Adding one requires product and security review plus an explicit catalog entry.

## Public action catalog
The launch catalog contains 76 retained actions: 73 default-authorized actions and 3 confirmation-required actions. Every retained action implicitly requires verified inside-Warp invocation and an exact-action credential. Every retained action must be implemented in the current branch; higher branches are not required to complete the catalog.

### Default-authorized inside Warp
These actions are safe enough to execute by default after enablement, proof, exact-action, and target checks succeed. They are non-destructive, do not require logged-in Warp-user authority, do not return potentially sensitive user data, and either modify reversible UI/reviewed configuration state or return minimized non-sensitive structural metadata. Persistence alone does not require confirmation when a change is reversible and stays within the reviewed allowlist.

Instance and app discovery:
- `instance.list`
- `instance.inspect`
- `app.ping`
- `app.version`
- `app.active`
- `app.focus`

Catalog and capability discovery:
- `capability.list`
- `capability.inspect`
- `action.list`
- `action.inspect`

Window actions:
- `window.list`
- `window.inspect`
- `window.create`
- `window.focus`

Tab actions:
- `tab.list`
- `tab.inspect`
- `tab.create`
- `tab.activate`
- `tab.move`
- `tab.rename`
- `tab.reset_name`
- `tab.color.set`
- `tab.color.clear`

Pane actions:
- `pane.list`
- `pane.inspect`
- `pane.split`
- `pane.focus`
- `pane.navigate`
- `pane.resize`
- `pane.maximize`
- `pane.unmaximize`
- `pane.rename`
- `pane.reset_name`

Session actions:
- `session.list`
- `session.inspect`
- `session.activate`
- `session.previous`
- `session.next`
- `session.reopen_closed`

Content-free block metadata:
- `block.list`

`block.list` may return only opaque block IDs, indices, status, timing, exit status, and similarly reviewed structural metadata. It must not return command text, output, working directories, environment values, or other block contents.

Theme and appearance actions:
- `theme.list`
- `theme.get`
- `theme.set`
- `theme.system.set`
- `theme.light.set`
- `theme.dark.set`
- `appearance.get`
- `appearance.font_size.increase`
- `appearance.font_size.decrease`
- `appearance.font_size.reset`
- `appearance.zoom.increase`
- `appearance.zoom.decrease`
- `appearance.zoom.reset`

Reviewed settings and keybinding actions:
- `setting.list`
- `setting.get`
- `setting.set`
- `setting.toggle`
- `keybinding.list`
- `keybinding.get`

Surface actions:
- `surface.settings.open`
- `surface.command_palette.open`
- `surface.command_search.open`
- `surface.warp_drive.open`
- `surface.warp_drive.toggle`
- `surface.resource_center.toggle`
- `surface.ai_assistant.toggle`
- `surface.code_review.toggle`
- `surface.left_panel.toggle`
- `surface.right_panel.toggle`
- `surface.vertical_tabs.toggle`

File intent actions:
- `file.open`

Input-buffer staging:
- `input.insert`
- `input.replace`

### Explicit confirmation required
These actions require verified inside-Warp invocation, an exact-action credential, and a one-shot in-app confirmation.

Destructive actions:
- `window.close`
- `tab.close`
- `pane.close`

### Removed from the public catalog
The following 22 actions are not present in `ActionKind`, discovery, generated help, completions, or public operator documentation.

Authenticated-user flows:
- `auth.status`
- `auth.login`

Terminal block content, history, and open-file reads:
- `block.inspect`
- `block.output`
- `history.list`
- `file.list`

Disallowed input actions:
- `input.get`
- `input.clear`
- `input.mode.set`
- `input.run`

Direct Warp Drive actions:
- `drive.list`
- `drive.inspect`
- `drive.open`
- `drive.notebook.open`
- `drive.env_var_collection.open`
- `drive.object.share.open`
- `drive.object.create`
- `drive.object.update`
- `drive.object.delete`
- `drive.object.insert`
- `drive.object.share_to_team`
- `drive.workflow.run`

The catalog also excludes accepted-command submission, agent-prompt submission, local file content operations, AI conversation reads, environment-variable reads, arbitrary settings, debug actions, auth-token helpers, crash helpers, and arbitrary internal dispatch.

## Direct action constraints
### Structural metadata
Without confirmation, structural list and inspect actions may return opaque IDs, indices, active/focused state, target types, counts, and reviewed operational metadata. They must not return terminal commands, output, input buffers, history, environment values, working directories, file paths, AI content, Drive content, or other user-authored content.

### Input staging
`input.insert` and `input.replace` stage plain text as a reversible input-buffer change. They must reject newlines, carriage returns, Enter-equivalent input, terminal control sequences, and any mechanism that submits or executes input. Changing input mode and clearing or reading the buffer are not public actions.

### Settings
`setting.list` and `setting.get` expose only an explicit non-secret, local, non-security-sensitive allowlist. `setting.set` and `setting.toggle` may mutate only reviewed reversible local settings and are default-authorized even when the change persists. Warp control enablement, authentication, account/team state, private/internal settings, security settings, cloud-backed settings, irreversible settings, and settings that widen another control surface are excluded.

### Creation actions
`window.create` and `tab.create` may create only reviewed logged-out-capable local session types. They cannot create cloud-agent sessions or otherwise invoke a logged-in Warp-user capability.

### Surface actions
Surface actions may show, focus, or toggle the existing UI. They must not return underlying Drive, AI, code-review, or other surface content; execute content; or mutate authenticated/cloud-backed data.

### File actions
`file.open` is an app-state intent that opens a caller-supplied path in Warp. It does not read file content. Open-file listing, arbitrary filesystem traversal, and file-content operations remain excluded.

## Targeting model
Target selection is hierarchical within the proof-issuing app instance:
- Window selectors resolve inside the instance.
- Tab selectors resolve inside the window.
- Pane selectors resolve inside the tab or pane-group context.
- Session selectors resolve inside the pane.
- Block selectors resolve inside the terminal session and are usable only by content-free `block.list`.
- File paths and visible surfaces are orthogonal instance-scoped targets.

Every selector family supports opaque IDs returned by safe discovery. Active, scoped index, exact title/name, and path selectors may be offered only where their results remain deterministic and do not expose excluded content. Explicit IDs that disappear fail with `stale_target`; zero matches fail with `missing_target`; multiple matches fail with `ambiguous_target`.

## CLI and documentation conventions
The typed action catalog is the source of truth for action names, support status, target scope, typed parameters/results, and `requires_user_confirmation`. Generated help, completions, reference docs, the built-in Warp Agent skill, and capability discovery must not mention removed actions as available. All 76 retained actions must be marked implemented before the current branch is complete.

The built-in skill should teach agents to prefer native tools for file content, shell execution, and attached context; use `warpctrl` only for visible Warp UI control; and explain that confirmation-required actions cannot be automated.

## Delivery implications
The current branch owns the complete launch contract:
- verified terminal proof, inside-Warp-only credential issuance, exact-action enforcement, and one-shot confirmation;
- every default-authorized read, target, UI, appearance, configuration, surface, and file-intent handler;
- every confirmation-required destructive handler;
- generated help, completions, capability metadata, documentation, and tests for the complete retained catalog.

Useful handler work from higher branches may be folded into the current branch, but no higher branch is required for launch. Obsolete higher branches may be dropped rather than restacked. The current branch must not add Drive mutation, workflow execution, command execution, terminal-content reads, or other removed actions.

Outside-Warp invocation, authenticated Warp-user actions, terminal-content reads, direct Warp Drive operations, and execution actions require separate future product and security review and an explicit catalog change; they are not incremental extensions implicitly authorized by this design.
