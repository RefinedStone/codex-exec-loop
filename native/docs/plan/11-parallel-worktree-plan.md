# Parallel Worktree Plan

This document splits the remaining native work into worktree-ready slices so multiple branches can move at the same time without colliding.

Status baseline:
- snapshot date: 2026-04-08
- default merge target: `prerelease`
- source backlog: `docs/plan/02-todo-backlog.md`, `../TODO.md`, and `plan/10-inline-scrollback-shell.md`

This file covers the work that is still open or only planned. It is intentionally more detailed than the baseline roadmap docs because the point is to make concurrent execution safe.

## 1. Planning Rules

- one lane can have several future slices, but only one active worktree in that lane at a time unless the write sets are clearly disjoint
- every branch below assumes a fresh branch from `origin/prerelease` unless a dependency is called out
- assume another unmerged worktree may already exist and check that before opening a new branch
- use branch names in the form `kind/native-<lane>-<zone>-<slice>` so another worker can infer the likely ownership area
- when a slice touches a hotspot file from `04-worktree-branch-rules.md`, do not start another hotspot-heavy slice in parallel without an explicit decision
- every slice below should ship with its own tests or documentation updates instead of relying on a later cleanup branch

## 2. Preflight For Another Worker

Before starting a new worktree:

- run `git worktree list`
- run `git branch -vv`
- inspect open PRs when GitHub access is available
- identify which lane and zone are already occupied
- prefer a disjoint lane or zone when two workers are active
- if overlap is intentional, record the expected conflict files in the task note or PR body

## 3. Two-Worker Default Model

The default concurrency target is two code workers. A third worktree is acceptable only for docs or validation sidecars.

Preferred pairings:

- one hotspot-heavy lane plus one disjoint support lane
- one runtime or shell lane plus one session, follow-up, GitHub, or docs lane
- `F1` can run as a low-risk sidecar with almost any other active slice

Good two-worker pairings:

- `A1` with `B1`
- `A1` with `C1`
- `B1` with `E1`
- `C1` with `E1`

## 4. Recommended First Wave

These slices can start together with the lowest collision risk.

| Lane | Branch | Goal | Main Files |
| --- | --- | --- | --- |
| A1 | `feature/native-shell-runtime-split` | extract the inline-shell runtime seam without changing product behavior yet | `app/app_runtime.rs`, `app/shell_frontend.rs`, new runtime/frontend modules |
| B1 | `feature/native-followup-max-turns-edit` | add editable max-auto-turn controls in the native shell | `app/conversation_model.rs`, `app/followup_controls.rs`, `app/followup_overlay_ui.rs` |
| C1 | `feature/native-session-query-model` | add session search, paging state, and recent-project filter model | `application/service/session_service.rs`, `app/session_overlay_ui.rs`, session presentation helpers |
| E1 | `feature/native-github-poller-port` | define the GitHub polling boundary and adapter without wiring the full UI yet | `application/port/*`, `application/service/*`, `adapter/outbound/*` |
| F1 | `docs/native-platform-validation-matrix` | create the validation matrix for Windows and macOS terminal behavior | `docs/*`, packaging or validation notes |

Avoid starting `D1` in the first wave because it shares the same runtime hotspot area as `A1`.

## 5. Lane A: Inline Scrollback Shell Migration

This lane maps the large refactor already described in `plan/10-inline-scrollback-shell.md`.

Only one active worktree from lane A should exist at a time because most slices touch `app_runtime.rs`, `shell_rendering.rs`, or `shell_presentation.rs`.

### A1. Runtime and Frontend Split

- branch: `feature/native-shell-runtime-split`
- status: ready to start
- goal: separate shared shell runtime behavior from frontend-specific rendering setup
- ownership: `src/adapter/inbound/tui/app/app_runtime.rs`, `src/adapter/inbound/tui/app/shell_frontend.rs`, new modules under `src/adapter/inbound/tui/app/`
- done when:
  - the runtime owns effect execution and background-message handling without depending on one renderer shape
  - alternate-screen and main-buffer setup are explicit frontend choices
  - current behavior still passes existing tests
- verify with:
  - `cargo fmt`
  - `cargo build`
  - `cargo test`

### A2. Presentation Neutralization

- branch: `feature/native-shell-presentation-neutral`
- status: after A1
- goal: move reusable text summaries away from fullscreen-only legends and titles
- ownership: `shell_presentation.rs`, `shell_layout.rs`, targeted tests in `app.rs`
- depends on: A1
- done when:
  - presentation helpers can feed both inline and alternate-screen frontends
  - fullscreen-only strings no longer block an inline frontend

### A3. Inline Live Region Renderer

- branch: `feature/native-shell-live-region`
- status: after A2
- goal: introduce an inline live-region renderer while keeping alternate-screen as fallback
- ownership: `shell_rendering.rs`, new renderer modules, `transcript_viewport.rs`
- depends on: A2
- done when:
  - main-buffer mode no longer redraws the entire frame as if it were fullscreen
  - alternate-screen behavior still works

### A4. Inline Inspection Surfaces

- branch: `feature/native-shell-inline-inspection`
- status: after A3
- goal: replace modal diagnostics, sessions, and template overlays in main-buffer mode with inline inspections
- ownership: `session_overlay_ui.rs`, `followup_overlay_ui.rs`, `shell_controller.rs`, `shell_rendering.rs`
- depends on: A3
- done when:
  - main-buffer mode no longer depends on popup-first interaction for diagnostics, sessions, or templates
  - current capabilities remain accessible

### A5. Scrollback-Safe Streaming History

- branch: `feature/native-shell-stream-history`
- status: after A3
- goal: make streaming output append into stable history without replaying the entire shell frame
- ownership: `conversation_runtime.rs`, `shell_rendering.rs`, `shell_presentation.rs`, transcript-related tests
- depends on: A3
- done when:
  - terminal scrollback reads like a coherent session
  - buffered manual input and auto follow-up behavior remain understandable while streaming

## 6. Lane B: Automation Controls and Visibility

Lane B can run in parallel with lane A until a slice needs `shell_presentation.rs` or shared runtime hotspots.

### B1. Editable Max Auto Turns

- branch: `feature/native-followup-max-turns-edit`
- status: ready to start
- goal: add UI editing for the maximum auto-follow turn count
- ownership: `conversation_model.rs`, `followup_controls.rs`, `followup_overlay_ui.rs`, related tests
- done when:
  - the operator can edit the max-turn value from the shell
  - invalid values are rejected predictably
  - the chosen value is visible in follow-up status output

### B2. Template Reload Action

- branch: `feature/native-followup-template-reload`
- status: after B1
- goal: reload workspace templates from the shell without restarting the app
- ownership: `followup_controls.rs`, follow-up service or filesystem adapter glue, overlay key hints
- depends on: B1 only if both slices would otherwise fight over the same overlay UI files
- done when:
  - a reload action exists in the shell
  - status text makes success, no-op reloads, and load failures explicit

### B3. Auto-Follow Activity Clarity

- branch: `feature/native-followup-activity-clarity`
- status: after B2
- goal: show clearer queue, submit, stop, and skip decisions around auto follow-up
- ownership: `conversation_runtime.rs`, `conversation_model.rs`, `shell_presentation.rs`
- depends on: B2
- done when:
  - queueing, submit, stop-rule hits, and skip reasons read clearly from the shell without digging into tests

## 7. Lane C: Session Browser Improvements

Lane C can run in parallel with lanes A, B, E, and F until lane A reaches inline inspection changes.

### C1. Query and Paging Model

- branch: `feature/native-session-query-model`
- status: ready to start
- goal: add search query, paging state, and recent-project filter state for recent sessions
- ownership: `application/service/session_service.rs`, `session_overlay_ui.rs`, session mapping or presentation helpers
- done when:
  - the state model supports filtering and paging without mixing UI concerns into domain types
  - regression tests cover state transitions and mapping behavior

### C2. Session Browser Controls

- branch: `feature/native-session-ui-controls`
- status: after C1
- goal: wire keyboard controls and visible filter summaries into the shell
- ownership: `shell_controller.rs`, `session_overlay_ui.rs`, `shell_presentation.rs`, tests in `app.rs`
- depends on: C1
- done when:
  - the operator can edit query text, move pages, and switch recent-project filters
  - the current filter state is visible from the session browser itself

### C3. Result Shaping and Empty States

- branch: `feature/native-session-result-shaping`
- status: after C2
- goal: improve ranking, empty-state copy, and recent-project context in the session browser
- ownership: session presentation helpers, session overlay tests, any small service shaping needed
- depends on: C2
- done when:
  - no-result, one-result, and multi-page states all read clearly

## 8. Lane D: Runtime Continuity and Activity Panels

Lane D should start after A1 lands because it shares the shared-runtime boundary and warning flow.

### D1. Shared Runtime Request Policy

- branch: `feature/native-runtime-request-policy`
- status: after A1
- goal: make concurrent fallback behavior explicit while a streaming turn holds the shared runtime
- ownership: `adapter/outbound/codex_app_server_adapter.rs`, runtime request tests, shell runtime glue
- depends on: A1
- done when:
  - concurrent startup, session, snapshot, or fallback requests have an explicit rule instead of incidental behavior
  - warning and retry behavior remain visible

### D2. Approval and Tool Activity Surface

- branch: `feature/native-runtime-activity-surface`
- status: after D1
- goal: show approval requests and tool activity in the live shell
- ownership: runtime event models, outbound adapter parsing, `shell_presentation.rs`, `shell_rendering.rs`
- depends on: D1
- done when:
  - approval and tool activity are visible without digging into raw stream events
  - activity does not hide auto follow-up state

### D3. Reconnect and Warning Visibility

- branch: `feature/native-runtime-warning-visibility`
- status: after D2
- goal: normalize reconnect, reset, and warning visibility in the shell
- ownership: outbound adapter notices, shell status output, regression tests
- depends on: D2
- done when:
  - warning transitions are readable and predictable
  - reconnect and reset messages do not disappear behind unrelated UI changes

## 9. Lane E: GitHub Review Polling

Lane E can start early because it primarily adds a new outbound boundary and app-facing state.

### E1. Polling Port and Adapter

- branch: `feature/native-github-poller-port`
- status: ready to start
- goal: define the application port and outbound adapter for GitHub PR review/comment polling
- ownership: new files under `application/port/`, `application/service/`, and `adapter/outbound/`
- done when:
  - the polling contract exists without leaking transport details into domain code
  - the adapter has unit coverage for response parsing and poll-state mapping

### E2. Runtime Poll Scheduling

- branch: `feature/native-github-poll-scheduling`
- status: after E1
- goal: integrate the poller with the native runtime lifecycle and app state
- ownership: `app_runtime.rs`, shell runtime glue, app state or reducer modules
- depends on: E1
- done when:
  - the app can start, stop, and refresh polling on a predictable schedule
  - failures are visible without destabilizing the conversation runtime

### E3. Review Change Surface

- branch: `feature/native-github-ui-notices`
- status: after E2
- goal: surface new PR review and comment changes in the shell
- ownership: `shell_presentation.rs`, `shell_rendering.rs`, relevant app tests
- depends on: E2
- done when:
  - review changes appear as clear notices in the native UI
  - the notices fit the rest of the shell without hijacking the main conversation flow

## 10. Lane F: Platform Validation and Packaging

Lane F is the safest sidecar lane and should stay mostly independent from hot runtime files.

### F1. Validation Matrix

- branch: `docs/native-platform-validation-matrix`
- status: ready to start
- goal: document how native raw-mode, alternate-screen, and inline-shell behavior should be checked on Windows and macOS
- ownership: `docs/`, validation notes, operator checklists
- done when:
  - the repo has one canonical validation matrix for terminal behavior
  - manual validation steps are specific enough to reuse during future PRs

### F2. Windows Compatibility Fixes

- branch: `fix/native-platform-windows-compat`
- status: after F1 and only when concrete findings exist
- goal: land any Windows-specific raw-mode or terminal behavior fixes found during validation
- ownership: frontend/runtime files only as required by validated findings
- depends on: F1
- done when:
  - validated Windows issues are fixed with focused regression coverage or documented manual checks

### F3. Packaging and Operator Docs

- branch: `chore/native-platform-packaging`
- status: after F1
- goal: define packaging steps and operator-facing run/install documentation for macOS and Windows
- ownership: packaging scripts, `README.md`, release notes or operator docs
- depends on: F1
- done when:
  - packaging steps are documented and repeatable
  - operator docs match the current native product story

## 11. Parallelism Guardrails

Do not run these slices at the same time:

- A1 with D1
- A3 with C2
- A4 with C2 or C3
- B3 with D2
- E2 with A1
- E3 with A3 or D2

Reason:
- each pair shares the same runtime, renderer, or presentation hotspots and would create noisy rebases

## 12. Handoff Rule

When one slice finishes:

1. rebase it onto the latest `origin/prerelease`
2. fast-forward `prerelease` to that reviewed head
3. close the PR after `prerelease` contains the commits
4. remove the finished worktree
5. start the next slice in that lane from the updated `origin/prerelease`

If the lane definition changes because a precursor extraction lands or the product direction changes, update this file first and then open the next worktree.
