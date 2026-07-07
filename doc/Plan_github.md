# GitHub Integration Plan & Status (Bang)

Status of the GitHub integration for the Bang fork, and the plan for the
remaining work. Spans two repos:

- **Client**: `~/Workspace/Warped/warp` (the Bang app, Rust)
- **Backend**: `~/Workspace/Warped/harness-backend` (Node/Express/Mongo stand-in), deployed at **https://api.trybang.ai** on Railway (auto-deploys on push to `main`)

## Context

Goal: GitHub integration at Cursor's level, phased across four milestones (G1-G4),
with plan-tier governance mirroring the MCP-marketplace pattern (tier entitlement +
workspace admin settings + DB-checked roles). The client already had strong
foundations (gh CLI plumbing, PR metadata models, code review UI, OAuth infra), so
G1 built on those rather than greenfield.

## Milestones

| Milestone | Scope | Status |
|---|---|---|
| **G1** | Foundation: GitHub App auth, direct API client, agent GitHub actions, PR-aware UI | **Shipped** (connect verified live) |
| **G2** | Event-driven automations: webhook engine + queue + Docker worker + automations UI | **Shipped** (client + backend; contract reconciled) |
| **G3** | Bugbot: automatic PR review with inline comments + auto-fix | **Not started** |
| **G4** | Enterprise controls: repo allowlists, org locking, audit/SIEM | **Partial** (governance plumbing exists; audit + full enforcement pending) |

## What's shipped and verified

- **GitHub App** `trybang` registered (App ID `4230291`); org-level install; permissions metadata:read, contents:read, pull_requests:write, issues:read; events installation(_repositories), pull_request, issues, issue_comment.
- **Backend (G1)**: App JWT + installation-token minting, webhook receiver (HMAC verify + delivery dedup), OAuth + `/github/setup` install-claim (org-locked: one workspace per GitHub org), `/api/v1/github/token`, real `UserGithubInfo`, tier `githubPolicy` + `WorkspaceSettings.githubSettings`. Verified live: signed webhook accepted (202), bad signature rejected (401), duplicate delivery de-duped (200).
- **Client (G1)**: `github_integration` feature (now **default-on**); `crates/github_client` (typed reqwest client); `GithubConnection` singleton + `get_github_token`; Settings > GitHub connect page; `GitHubRepoModel::ApiBacked` with PR chip + checks; read-only PR review-comment overlay. Six agent actions (ReadGithubPr, ListGithubPrComments, CreateGithubPr, ReadGithubIssue, ListGithubIssues, ReplyToPrComment) via the `warp-proto-apis` fork (`therealinvoker/warp-proto-apis`, rev `d6b7b19`, pinned in workspace Cargo.toml).
- **G2 automations**: backend engine (`github/events.js` normalization, Mongo-backed `AgentRun` queue with atomic claim + reaper, Docker worker + claude harness adapter, full ambient-agent REST surface); client automations settings UI + provider-key admin + run visibility.
- **Contract reconciliation (backend commit `b2504ea`, deployed)**: the G2 client and backend were built independently and diverged. Fixed backend to match client at the GraphQL boundary (internal multi-event engine unchanged):
  - `trigger`: serve/accept `eventType` + `repoFilter` (mapped to internal `events[]`/`repoFullNames[]`)
  - `action`: `actionType`<->`kind`, `modelId`<->`model`
  - `ListGithubAutomationsOutput` includes `providerKeys`
  - `SetGithubProviderKeyOutput` returns single `providerKey`; `RemoveGithubProviderKeyOutput` returns `success`
  - Verified 113/113 unit tests + a round-trip test. Redeem, MCP governance/allowlist CRUD, and `GetUpdatedCloudObjects/mcpGallery` were already matching.
- **Live connect works**: OAuth authorize -> callback -> token stored -> "Connected as <user>". Also fixed a Settings > GitHub crash (warp commit `e19b09f5`: `MainAxisSize::Max` -> `Min` in `github_page.rs`) and the `addonCreditsOptions` non-null field on `PricingInfo` that was breaking `GetWorkspacesMetadataForUser` (backend, deployed).

## Deploy & config (Railway)

Env vars on the `api` service (set via `railway variables`):
`PUBLIC_URL=https://api.trybang.ai`, `SECRETS_KEY` (64 hex), `GITHUB_APP_ID`,
`GITHUB_APP_SLUG=trybang`, `GITHUB_WEBHOOK_SECRET`, `GITHUB_OAUTH_CLIENT_ID`,
`GITHUB_OAUTH_CLIENT_SECRET`, `GITHUB_PRIVATE_KEY_BASE64` (base64 of the .pem;
NOT a path — Railway has no persistent file). Backend degrades gracefully with
any of these unset ("GitHub integration not configured" is a per-server-instance
message, not per-tenant). Full setup: `harness-backend/docs/github-app-setup.md`.

## Next: Task 17 — single-step connect (Option B)

**Problem**: connecting is two GitHub visits today (authorize, then separately install/pick repos). Collapse into one.

**Approach**: use GitHub's native OAuth-during-installation so one flow authorizes AND installs. Three coordinated parts, must ship together:
1. **App config**: enable "Request user authorization (OAuth) during installation" on the `trybang` App.
2. **Client** (`app/src/settings_view/github_page.rs`): `connect()` opens the install URL (`app_install_link`) instead of `/github/oauth/start`; keep the `next=` deep-link (`github_auth_url.rs`).
3. **Backend** (`src/routes/githubOauth.js` / `githubWebhooks.js`): `/github/setup` handles the combined callback — GitHub redirects with both `installation_id` and OAuth `code`; claim the installation and exchange the code (store token) in one handler.

**Result**: one "Connect GitHub" click -> one GitHub page (authorize + pick repos) -> back in the app, connected and installed.

**Sequencing/risk**: flip the App checkbox as the code deploys, not before (it changes the live flow and would break the current two-step connect mid-testing). Backend change = one prod push when ready. Verify end-to-end against a test repo.

## Remaining after Task 17

- **G3 Bugbot**: on PR opened/pushed, sandboxed review run -> structured findings -> inline PR review comments + optional auto-fix commits; per-repo enable + effort; backend-heavy. Needs App permission bump (contents:write, checks:write, workflow_run events) and installer re-approval.
- **G4 enterprise**: wire `githubPolicy` into the `GetWorkspacesMetadataForUser` client fragment (client `github_policy` currently deserializes to None / flag-only gating); repo allowlist + org-lock admin UI; audit log surfacing + SIEM streaming (currently deferred with a forwarder sketch).

## Known issues / follow-ups

- **Rotate the leaked OAuth client secret**: an earlier value passed through a chat transcript. Regenerate on the App and update Railway (`GITHUB_OAUTH_CLIENT_SECRET`).
- **`grep` in Claude Code sessions**: the shell wrapper routes `grep` to an embedded `ugrep -I` that misclassifies UTF-8-containing source files as binary and skips them (false "no match"). Not locale-fixable. Use `command grep`, `grep -a`, or the built-in Grep tool. This is a Claude Code bug worth reporting; unaffected in normal terminals.
- **Auto-deploy caution**: any push to harness-backend `main` deploys to prod. Commit locally and push deliberately.
- **Client automation shape**: keep client automation trigger/action fields (`eventType`/`repoFilter`/`actionType`/`modelId`) aligned with the backend (commit `b2504ea`) to avoid re-diverging the contract.

## Key files

**Client**: `crates/github_client/`, `app/src/github/{mod,pr_review_comments}.rs`, `app/src/server/server_api/integrations.rs`, `app/src/ai/ambient_agents/{github_auth_url,task}.rs`, `app/src/code_review/github_repo_model/`, `crates/ai/src/agent/action/` (+ `blocklist/action_model/execute/github.rs`), `app/src/settings_view/{github_page.rs, github_automations/}`, `crates/graphql/src/api/{billing,workspace}.rs`, `crates/warp_graphql_schema/api/schema.graphql`, `app/src/workspaces/{workspace,gql_convert}.rs`.

**Backend**: `src/github/{app,api,events,crypto,providerKeys}.js`, `src/models/{GithubInstallation,GithubAutomation,AgentRun,GithubAuditEvent,Workspace,tiers}.js`, `src/agents/{queue,launcher,serialize,bugbot?}.js`, `src/routes/{githubWebhooks,githubOauth,agentRuns,graphql}.js`, `src/graphql/{github,mcpGovernance,teams,workspaces}.js`, `worker/`.
