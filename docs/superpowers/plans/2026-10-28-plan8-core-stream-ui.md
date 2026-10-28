# Core-owned generation and chat layout implementation plan

> **For agentic workers:** Execute independent slices with subagents; the integration owner handles core streaming and final checks. Read Agent.md. Do not run builds/tests/formatters or commit mid-flight.

**Goal:** Implement the user's new-account password policy, sidebar role cards, compact themed composer, aligned message avatars, and browser-independent generation with refresh recovery.
**Architecture:** Rust owns one generation per conversation. HTTP clients observe snapshots and subsequent deltas; observer disconnect cannot cancel an upstream request. Explicit stop saves the interrupted reply; deleting a conversation cancels/discards it. Existing account storage modes remain unchanged.
**Tech stack:** Existing Rust/axum/SQLite/tokio and SolidJS/WASM, without replacing the rendering pipeline.
**Spec:** User's 2026-10-28 request and existing `docs/superpowers/specs/2026-10-26-nekochat-design.md`, updated to these semantics.

## Global constraints
- Preserve live data, roles, OAuth credentials and provider environment; no secret output. Do not change the administrator password.
- Default host deployment stays 0.0.0.0:10000 with Host checks.
- Only relevant regressions, one integrated build, isolated browser/API smoke; do not repeatedly run the full suite.
- No model/provider/effort labels in ordinary messages. Only role cards expose their public names in the sidebar.
- Owner validation covers generation observation and explicit stop, not only turn creation.
- Update and stage CHANGELOG before every commit; no merge or push.

## Review focus
- Refresh at the exact handoff from snapshot to delta must neither duplicate nor lose text.
- A slow or disconnected observer must not block generation or cancel it; simultaneous observers receive consistent results.
- A stale stop action must not cancel a newer generation, and deletion must not resurrect an assistant node.
- Logout/session revocation removes access but does not terminate the owner-bound generation; deleting the account deletes/cancels its conversations.
- Existing plaintext/hashed credentials and their sessions do not change when the new-user policy changes, including later login and password reset.

## Task 1: New-account policy and admin role default
**Owner/files:** PasswordAdmin owns `crates/store/src/users.rs`, `crates/store/tests/users.rs`, `crates/server/tests/accounts.rs`, `web/src/admin/Users.tsx`, `web/src/admin/Management.tsx`.
**Interfaces:** Keep existing API paths and `password_storage` setting, now explicitly a new-account default. Keep each user's `password_storage` on login and password reset. Preserve credential revision checks. Parent owns `server/src/accounts.rs` for generation lifetime.
- [x] Add consumer regression for unchanged old credentials and differing new-account modes; parent runs it before implementation.
- [x] Remove bulk conversion, login-time conversion and mode-change VACUUM; update password writes to the account's stored mode.
- [x] Update obsolete conversion tests to the new policy, including a session staying valid across policy changes.
- [x] Remove policy-change confirmation and clarify new-account-only labels; retain explicit password reveal and unrelated destructive confirmations.
- [x] Add a per-role-card “设为默认” action using existing settings API, availability checks and default indicator.

## Task 2: Sidebar and message layout
**Owner/files:** ChatLayout owns `web/src/components/Chrome.tsx`, `Pickers.tsx`, `Composer.tsx`, `Message.tsx`, and `web/src/styles.css`.
**Interfaces:** Existing `store.character/characters/chooseCharacter/loadCharacters` remain. Parent supplies `store.stopTurn(): Promise<void>`; stop click uses `void store.stopTurn()` instead of aborting an HTTP request.
- [x] Move the role chooser above “新对话” in the sidebar, with a full-width public role card; remove it from the composer entirely.
- [x] Style attachment as a small themed paperclip button using existing theme variables, preserving paste/drop/upload/caret behavior.
- [x] Remove visible per-message role/model names. Render assistant avatar left and user avatar right at the same top row as the first content line, including reasoning/attachments.
- [x] Keep mobile layout, virtualization, markdown worker, attachments and action controls functional; no per-token extra work.

## Task 3: Core-owned generation and reconnect
**Owner/files:** Parent owns `server/src/state.rs`, `chat.rs`, `api/user.rs`, `accounts.rs`, `lib.rs`, necessary new generation module, related stream tests, and `web/src/api.ts`/`store.ts`.
**Interfaces:** `POST /api/conversations/{id}/turns` starts generation. Add authenticated observation for the current generation and an explicit stop endpoint scoped to the visible turn ID. Expose a running indication on conversation reload. Snapshots include partial public node content/reasoning, role avatar and original start time; deltas follow the snapshot atomically.
- [x] Read current transport cancellation and add a regression proving dropping the initial observer must not stop upstream or prevent completion.
- [x] Replace receiver-closed cancellation with a core-owned run registry and bounded observer delivery; resynchronize lagging observers with a snapshot.
- [x] Keep cumulative content once in core, avoid copying full replies per delta; isolate HTTP serialization from transport-neutral events.
- [x] Implement explicit stop (persist interrupted partial) versus forced deletion (discard), preserving one-run-per-conversation until final persistence.
- [x] Remove logout/password/disable-driven generation cancellation; retain actual conversation/account deletion cancellation. Reject clear while generating rather than silently cancelling.
- [x] Frontend observes on reload/revisit, restores latest snapshot without duplication, detaches on navigation/logout, and stops only via explicit API. Handle completion racing with reconnect.

## Task 4: Integration and delivery
**Owner:** Parent.
- [x] Run relevant account and stream regressions, TypeScript/WASM/Vite and release build after integration.
- [x] Exercise actual UI, controlled streaming refresh/reconnect, explicit stop and deletion, ownership rejection and password policy in an isolated instance.
- [x] Update current design, Agent.md, README, CHANGELOG and package versions to 0.8.0.
- [x] Confirm live cutover timing, safely back up/restart the host service preserving provider environment and OAuth, then verify actual host access.

## Delivery evidence — 2026-10-28
- Related Rust runs covered users, accounts, conversations, role turns, turn lifecycle, lag recovery and revocation. The document-history regression exposed same-millisecond UUID ordering; reserving the user ID before its generation fixed it. Workspace/test type checking and production builds passed.
- Controlled HTTP/browser smoke used no paid models. Refresh and fully closing/reopening the page retained one upstream call and restored both prior and observer-free output exactly once. Explicit stop persisted interrupted content; failed DELETE 503 preserved observation, clear returned 409, successful forced DELETE returned 204 and subsequent reads 404.
- Session rotation and queued-preference races were reproduced in the browser (PATCH 200 followed by stale request 401/false logout), then passed after observer/request-epoch cleanup and a preference queue barrier (PATCH 200, preference 200, authenticated).
- Admin UI policy switching had no dialog; actual stored modes remained plain/hash after switching, login and reset. Per-card default saved successfully and did not override the existing user's remembered role. Desktop and 390px mobile surfaces were inspected.
- User approved immediate cutover after verification. Live `nekochat-live-0-8` PID 641380 is persistent on 0.0.0.0:10000; actual LAN root rendered, anonymous history returned 401, unknown Host 421.
- Backup: `/home/xy6/.local/state/nekochat/backups/pre-0.8.0-20261028T123226Z-534208` (private old executable/config/data). All 11 business tables and 9 conversations matched; configuration, OAuth bytes and configured provider environment were preserved.
