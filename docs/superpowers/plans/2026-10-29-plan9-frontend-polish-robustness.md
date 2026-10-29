# Frontend Polish and Robustness Implementation Plan

> **For agentic workers:** Use subagent-driven-development with the ownership and integration rulings below. Implement all four approved steps without another approval loop. Parent alone integrates, checks, updates CHANGELOG and commits.

**Goal:** Make the existing frontend visibly more refined while preserving reliable chat, account, attachment and administration behavior.
**Architecture:** Keep Rust core, SolidJS, WASM Markdown worker, virtualization, role authorization and current API contracts. Improve shared styling first; implement independent chat/admin/state slices with one owner per file; integrate and exercise real browser scenarios against an isolated controlled upstream.
**Tech Stack:** Existing Rust/axum/SQLite, SolidJS, CSS theme variables and Vite. No new UI framework, font service or runtime dependency.
**Spec:** User-approved four-step roadmap in this conversation (2026-10-29), plus `docs/superpowers/specs/2026-10-26-nekochat-design.md` and `Agent.md`.

## Global Constraints
- Remain on the existing feature branch. Do not merge, push, rotate credentials, change public Host policy or migrate production data for this task.
- Preserve Claude Dark default, all eight built-in themes and custom CSS themes. All colors use existing `--nk-*` variables; optional geometry variables belong to shared layout, not required theme files.
- Preserve sidebar role cards above New Chat; no model/provider/effort/plan labels in normal chat or composer. Assistant/user avatars remain on opposite sides, aligned with first content.
- Keep mobile text controls at least 16px, normal manual keyboard input, pinch zoom, safe-area padding and touch access. Do not add autofocus suppression or viewport zoom caps.
- Preserve inline image editing, above-editor document chips, slash-only command disclosure, actual document extraction and user ownership checks.
- Preserve one core-owned generation per conversation; disconnect/navigation/logout detach HTTP observers, only scoped explicit stop or destructive deletion cancel generation. Do not retry model POSTs.
- Preserve per-user state epochs, password-rotation preference barrier and authentication revocation. No broad exception swallowing or fake success.
- Do not add token-hot-path DOM scans, whole-message copies, continuous animations, heavy blur, external assets or new telemetry/retry infrastructure.
- Implementers skip builds, lint, tests, formatting, commits and nested agents. Parent performs targeted regression and actual browser smoke after integration. Existing UI benchmark may be used; no new test framework or source-text tests.
- Parent updates and stages CHANGELOG before every commit. Four phase commits are required; final release/version bookkeeping is parent-owned. Do not restart the live service per phase.

## Review Focus
- Rich-editor native empty BR/composition/image selection must not cause placeholder overlap, lost draft text or accidental send.
- HTTP/nonsecure clipboard and permission rejection must produce real success/failure without unhandled errors or lost selection.
- A disconnected observer must show honest reconnect state and recover the same upstream call, not duplicate content or POST.
- User scroll-up, delayed image layout and virtualized row measurement must not pull the reader back to the bottom.
- Long admin values, failed saves, disabled controls and narrow keyboard-height viewports must stay operable without page-wide horizontal overflow or draft loss.

## Ownership and Shared Interfaces

| Owner | Writable files | Contract |
|---|---|---|
| Parent / step 1 | `web/src/styles.css`, `components/Chrome.tsx`, `Account.tsx`, `Pickers.tsx`, `Popover.tsx`, `App.tsx`, `web/THEMES.md`, entry viewport metadata if necessary | Sole shared-CSS integration owner; own header/sidebar/account shell, theme semantics and dialogs |
| Chat worker / step 2 | `components/Composer.tsx`, `Message.tsx`, `MessageList.tsx`, `Files.tsx`, `TreePanel.tsx`; create `web/src/clipboard.ts` if required | Keep existing public exports/classes; consume LiveTurn connectionState; report added class hooks, never edit shared CSS |
| State worker / step 2+4 | `web/src/store.ts`, `web/src/api.ts` | Add connection status without changing existing transport/core lifetime, retry or auth policies |
| Admin worker / step 3 | `web/src/admin/**` except API/schema/monitoring transport changes | Own admin CSS and component markup; preserve actual save/default/password/resource behavior |
| Parent / step 4 | docs, version files, benchmark only if a real issue requires it, integration corrections | Controlled backend/browser proof, scoped independent review, release and credential-preserving deployment |

Shared geometry: `--nk-sidebar-width: 240px`, `--nk-content-width: 860px`, `--nk-radius-sm: 8px`, `--nk-radius-md: 12px`, `--nk-radius-lg: 18px`. Preserve color token names. Desktop controls should have a consistent compact size; primary touch actions target at least 44px without making all dense data rows oversized.

State worker produces `export type ConnectionState = "connecting" | "streaming" | "reconnecting"`; `LiveTurn.connectionState: Accessor<ConnectionState>` and `LiveTurn.set.connectionState: Setter<ConnectionState>`. `createLiveTurn` initializes connecting; start/snapshot and received events establish streaming; broken observation/GET retry marks reconnecting until a valid new snapshot. Stopping remains its existing separate signal and takes display priority. No per-delta extra allocation/timer is needed.

## Step 1 — Shared Visual Baseline

**Files:** Parent-owned shared files above.
- [x] Establish the isolated fixture, retain the immutable 85f3582 frontend, and capture comparable desktop/mobile, theme, editor and benchmark behavior.
- [x] Define optional geometry variables and refine sidebar/header/control hierarchy, spacing, borders, focus and popup treatment using existing palettes. Replace text-glyph trash/new-chat/menu controls with small currentColor SVG where it improves consistency.
- [x] Give account/login/loading/dialog surfaces the same card and field language; keep real pending/error states, form contents and native focus behavior. Restore popup trigger focus on Escape without stealing outside-click focus.
- [x] Ensure popups/dialogs fit narrow viewports and long text; preserve native scrolling and mobile input rules.
- [x] Correct theme documentation to match the actual Claude Dark default and document geometry as optional layout values.
- [x] Exercise desktop/mobile and representative dark/light theme families; stage parent baseline files (not worker files), update CHANGELOG, commit the phase after integration evidence is available.

## Step 2 — Chat Workspace and State Feedback

**Files:** Chat and state worker ownership above; parent integrates their CSS hooks.
- [x] Keep real markdown/worker/virtualizer flow; refine message actions, collapsed reasoning, debug usage labels, attachment layout and composer status/error presentation.
- [x] Reproduce and fix meaningful rich-editor issues only: native-empty editor normalization, composition boundaries, selection after paste/image removal and TreePanel's obsolete textarea focus target. Do not replace contenteditable with textarea or inject selection resets per token.
- [x] Make code-block copying work on HTTP with an actual clipboard implementation and honest failure feedback; preserve selection/focus and avoid copying the wrong block. Reuse this path for any affected copy actions.
- [x] Expose the shared connectionState contract from store and display connecting/reconnecting/stopping clearly. Do not change GET-only recovery or stopped/deleted semantics.
- [ ] Verify text/IME/multiline/clear, image+document input, tree editing, code-copy success/rejection, scroll-up under output, reconnect without duplicated POST and manual stop/delete. Stage chat/state files plus integrated chat CSS, update CHANGELOG, commit.

## Step 3 — Mobile and Administration

**Files:** `web/src/admin/**`; parent-owned mobile shared layout as needed.
- [x] Refine admin navigation, summary cards, tables, forms, local resource cards, file-selector controls and action hierarchy. Keep visual grouping clear instead of adding cosmetic wrappers everywhere.
- [x] Preserve role defaults, provider-specific model choices, source/usage information only in admin, new-account-only password storage and actual error handling.
- [x] Keep long labels/IDs/system prompts and directory rows inside cards; allow local table scrolling, full-width narrow fields and usable save/cancel actions.
- [x] Guard duplicate submits and draft loss during failed operations; correct only confirmed lifecycle/UI issues rather than adding unrelated validation or automatic retries.
- [ ] Exercise actual login, role editor, resource preview, model selection, user policy/defaults and failed saves on desktop/mobile and light/dark themes. Update CHANGELOG, commit admin phase independently.

## Step 4 — Integration, Robustness and Release

**Files:** parent integration, existing docs and release metadata.
- [x] Run the existing 500-message benchmark before/after under equivalent browser conditions; exercise a long individual response and input during output. Record observed frame/long-task data, not invented FPS promises.
- [x] Use a local controlled HTTP model for gated stream, reload/observer disconnect, offline reconnect, stop, failed delete, successful delete and known upstream error; no paid-model calls for tests.
- [x] Exercise account switching and invalidation without cross-user drafts/history, file upload failures without losing typed text, and clipboard failure without fake success.
- [x] Run relevant Rust account/conversation/turn regressions once, TypeScript/WASM/Vite and release build; fix only exercised failures and re-run the failed checks, not the entire suite repeatedly.
- [x] Obtain scoped independent spec/quality review of changed frontend/state/admin paths, fix evidence-backed findings and verify affected paths.
- [ ] Update README/theme guide/current design and version/CHANGELOG to the actual release. Commit the final phase with evidence and limits.
- [ ] Back up live binary/config/data, preserve provider environment/OAuth/Host allowlist and deploy once at the authorized final cutover; verify actual public URL and data preservation. No merge or push.

## Execution Rulings
- Existing feature checkout is intentionally retained: the user works and deploys from this branch, the live release embeds its assets, and all runtime proof uses a separate data directory. No unrequested worktree/branch/merge is introduced.
- Developer/tool contracts override generic skill boilerplate: independently owned worker files may be edited concurrently; workers do not run checks/formatters or commit. Parent is the integration owner for shared files and four commits.
- The approved roadmap is the design authority. This plan records exact interfaces and conservative visual decisions; it does not introduce extra features, a new backend architecture or a second design approval cycle.
- The unchanged snapshot permits faithful baseline capture after visual integration; final before/after benchmarks were rerun under the same 1360×833, DPR 1.25, non-touch Chromium conditions. Initial/after screenshots and native edit regressions remain separate evidence.
- Keep this already-approved branch and record four ordered commits; no merge, push, worktree removal or repeated full test suite is requested. Shared CSS, including hooks consumed by step 2, belongs to the step 1 visual foundation.
