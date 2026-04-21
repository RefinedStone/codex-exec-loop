# Current Product State

This file tracks the current branch product state. `origin/prerelease` already ships the first operator-facing supersession loop, and the current branch also includes the repo-scoped planning authority follow-through built on top of it.

## Product Identity

- The product is optimized for long-lived solo work sessions, not one-shot prompt submission.
- Inline shell is the primary surface and the host terminal scrollback is the durable history view.
- The distinctive value is continuity across startup diagnostics, session resume, planning files, and queue-driven auto follow-up inside one terminal.
- Planning is not an optional side panel anymore. Accepted planning state shapes follow-up preview, queue inspection, and post-turn automation.

## Shipped Surface

- Inline shell is the only frontend.
- Startup diagnostics begin immediately and the shell can render before all checks finish.
- Manual submit can queue while startup is pending, then auto-submit once the shell is ready.
- The client can start a new draft, resume a thread, load snapshots, and stream turns through the shared app-server boundary.
- Shell overlays cover diagnostics, recent sessions, queue-task and proposed-task inspection, automation controls, planning workspace controls, and direction-side authoring support.
- Recent sessions support search, paging, and current-workspace filtering.
- Auto follow-up is planning-queue-driven and no longer loads workspace-defined prompt files.
- Planning is live: `:planning` stages drafts, opens the embedded editor, promotes accepted files, and exposes the current queue task and proposed tasks.
- Shared planning lifecycle commands are live: `akra doctor`, `akra init`, `akra reset`, `:doctor`, `:init`, and `:reset` all use the same validation, bootstrap, and reset rules.
- Invalid planning writes are rolled back and may trigger a bounded repair flow.
- Runtime warnings, approval-review state, tool activity, and optional GitHub review polling are visible in normal shell flow.

## Current Operating Model

- Startup model:
  Startup diagnostics establish whether the shell can submit, browse sessions, and start planning-driven work.
- Session model:
  The operator either starts from a blank draft or resumes an existing thread through the session browser.
- Planning model:
  The operator owns accepted planning through staged drafts and explicit promotion.
- Automation model:
  Post-turn automation only acts on accepted planning, current queue task availability, and explicit stop rules.
- Recovery model:
  Invalid planning changes are restored, archived, and surfaced back to the operator instead of being silently accepted.

## Supersession Status

- `origin/prerelease` already ships readiness gating, the supersession control tower, the three-slot `akra` worktree pool, `reported_complete` versus official completion, and serial distributor delivery.
- The current branch also routes git-backed planning authority through a repo-scoped SQLite store under `.codex-exec-loop/runtime/`, including active planning, staged drafts, official refresh claims, distributor queue claims, and runtime slot, session, and distributor projections.
- Store-backed restart recovery now rechecks worktree, branch, and PR truth before reclassifying in-flight distributor work, and authority inspection can repair exported planning files from store truth when they drift or disappear.
- `auto stop` remains an automation control, not the supersession completion contract. When a leased parallel session reaches official completion, the same slot session stops and hands control back to the supervisor or distributor instead of reusing that slot session for another auto-follow turn.
- The main remaining supersession work is validation depth, compact doc alignment, and residual surface polish rather than the core loop or repo-scoped authority migration.

## Active Constraints

- Real-terminal validation is still required after prompt, streaming, overlay, or restore changes.
- Some shell actions remain gated by startup diagnostics.
- Non-stream requests can still fall back to isolated runtime access while a main stream is active.
- Planning detail mode supports manual authoring only; the `llm-assisted` branch remains disabled.
- The checked-in schema snapshot still predates newer app-server approval response methods, so the TUI does not expose approve or deny actions yet.

## Current Product Risks

- The operator mental model spans startup state, shell state, accepted planning, the current queue task, proposed tasks, repair state, and auto-follow state at once.
- Planning is powerful but still expensive to author for a casual workspace because staged drafts, promotion, queue-idle policy, and direction-side authoring concepts arrive early.
- Auto follow-up pause reasons are technically accurate but still read like system state rather than operator guidance.
- Overlay coverage is broad, but the current surface does not always make it obvious what the next recoverable action is.
- Approval review is visible but still requires out-of-band action.

## Code Entry

- Shell runtime entrypoint: `src/adapter/inbound/tui/app.rs`
- Supersession shell entrypoint: `src/adapter/inbound/tui/app/parallel_mode.rs`
- Supersession application services: `src/application/service/parallel_mode_service.rs`
- Planning authority port: `src/application/port/outbound/planning_authority_port.rs`
- Planning authority adapter: `src/adapter/outbound/sqlite_planning_authority_adapter.rs`
- Planning feature entrypoint: `src/adapter/inbound/tui/app/planning/`
- Application planning services: `src/application/service/planning/`
