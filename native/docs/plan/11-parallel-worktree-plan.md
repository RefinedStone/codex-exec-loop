# Parallel Worktree Plan

This document now records which slices from the first concurrent native delivery pass are already landed and keeps detailed planning only for the slices that still remain.

Unless noted otherwise, file paths below are relative to `native/`.

Status baseline:
- snapshot date: `2026-04-09`
- current reference branch: `origin/prerelease`
- source backlog: `docs/plan/02-todo-backlog.md`, repository root `../TODO.md`, and `docs/plan/10-inline-scrollback-shell.md`
- compaction rule: landed slices stay short here; only remaining slices keep detailed execution notes
- primary remaining work: lane A `A6` terminal-flow shell reset, then conditional platform follow-through in `F2`

## 1. Planning Rules

- one lane can have several future slices, but only one active worktree in that lane at a time unless the write sets are clearly disjoint
- every new slice assumes a fresh branch from `origin/prerelease` unless a dependency is called out
- assume another unmerged worktree may already exist and check that before opening a new branch
- use branch names in the form `kind/native-<lane>-<zone>-<slice>` so another worker can infer likely ownership
- when a slice touches a hotspot file from `docs/plan/04-worktree-branch-rules.md`, do not start another hotspot-heavy slice in parallel without an explicit decision
- every remaining slice should still ship with its own tests or docs instead of relying on a cleanup branch later

## 2. Preflight For Another Worker

Before starting a new worktree:

- run `git worktree list`
- run `git branch -vv`
- inspect open PRs when GitHub access is available
- identify which lane and zone are already occupied
- prefer a disjoint lane or zone when two workers are active
- if overlap is intentional, record the expected conflict files in the task note or PR body
- remember that only the integration checkout should own local `prerelease`; feature worktrees rebase onto `origin/prerelease`

## 3. Completion Snapshot

The slices below are already landed on `prerelease` and should stay compact in this file.

- [x] A1 Runtime and Frontend Split: shared shell runtime and frontend setup are separated, frontend selection is explicit, and terminal restore behavior has follow-up coverage.
- [x] A2 Presentation Neutralization: shared shell presentation copy was neutralized so inline and alternate-screen frontends can share more status and title helpers.
- [x] A3 Inline Live Region Renderer: inline main-buffer rendering landed while alternate-screen remains available as fallback.
- [x] A4 Inline Inspection Surfaces: inline mode now renders diagnostics, sessions, and follow-up templates as in-shell inspection surfaces with focused rendering coverage, while alternate-screen keeps the popup fallback.
- [x] A5 Scrollback-Safe Streaming History Foundation: active streaming output stays in the live region until completion, while thread and turn lifecycle markers commit into stable transcript history instead of replaying each delta as transcript state.
- [x] B1 Editable Max Auto Turns: the shell can edit max auto turns with bounded validation.
- [x] B2 Template Reload Action: workspace follow-up templates can reload from the shell without restarting.
- [x] B3 Auto-Follow Activity Clarity: queue, submit, stop, and skip outcomes are surfaced as clearer operator-visible activity.
- [x] C1 Query and Paging Model: recent sessions now carry native query, paging, and recent-project filter state.
- [x] C2 Session Browser Controls: the session browser exposes query editing, paging shortcuts, and visible filter state.
- [x] C3 Result Shaping and Empty States: session ranking, empty-state copy, and recent-project context were refined.
- [x] D1 Shared Runtime Request Policy: concurrent startup, session, snapshot, and fallback requests now follow explicit shared-runtime rules.
- [x] D2 Approval and Tool Activity Surface: approval review state and tool activity are visible from shell status output.
- [x] D3 Reconnect and Warning Visibility: reconnect, reset, runtime notice, and warning summaries were normalized.
- [x] E1 Polling Port and Adapter: the GitHub PR polling boundary and adapter are in place with response parsing coverage.
- [x] E2 Runtime Poll Scheduling: polling lifecycle, scheduling, and error visibility are wired into the native runtime.
- [x] E3 Review Change Surface: review/comment change notices now appear in the shell.
- [x] F1 Validation Matrix: the repository has a canonical terminal validation matrix for macOS and Windows.
- [x] F3 Packaging and Operator Docs: packaging, checksum verification, and operator handoff docs are in place.
- [x] M1 Root README Migration Cleanup: the repository root README is now native-first and keeps Python CLI guidance in a compatibility-only section.

Additional landed follow-ups that were not explicit slices in the original lane table:

- [x] startup-pending manual prompt submission queue
- [x] transcript jump shell commands
- [x] native package checksum and verification helper follow-ups
- [x] platform validation result capture helpers
- [x] WSL validation capture context and optional IDE terminal summary coverage
- [x] platform validation record directory and naming rules
- [x] platform validation coverage summary helper
- [x] platform validation markdown report helper

## 4. Remaining Slices

### A6. Tail-Anchored Terminal Flow Shell

- status: remaining
- branch: `feature/native-shell-terminal-flow`
- goal: remove the dedicated middle `Transcript / tail` viewport from inline mode so the host terminal becomes the primary history surface and the tail prompt/live region becomes the only anchored shell region
- ownership: `src/adapter/inbound/tui/app/ratatui_frontend.rs`, `src/adapter/inbound/tui/app/shell_rendering.rs`, `src/adapter/inbound/tui/app/shell_presentation.rs`, `src/adapter/inbound/tui/app/shell_controller.rs`, `src/adapter/inbound/tui/app/transcript_viewport.rs`, and shell rendering tests under `src/adapter/inbound/tui/app/`
- depends on: A1 through A5 foundation
- landed follow-ups inside A6 so far:
  - inline shell chrome collapsed into transcript body plus one tail prompt region
  - inline transcript is pinned to tail and viewport commands no longer steer the inline path
  - tail prompt guidance is compact and no longer prints a dedicated `Prompt / ...` title row
- current blocker:
  - `src/adapter/inbound/tui/app/ratatui_frontend.rs` still runs `terminal.draw(...)` every loop
  - `src/adapter/inbound/tui/app/shell_rendering.rs` still redraws inline transcript plus tail as one visible frame
  - the result is that some terminals still show repeated redraw or replay-like scrollback even though most of the old shell chrome has been removed
- done when:
  - inline mode no longer repaints stable shell output as one repeated frame
  - sequential history reads top-to-bottom like Codex CLI or a Spring Boot application log
  - host terminal scroll or mouse-wheel behavior is the primary way to inspect older output in inline mode
  - a tail-anchored prompt box remains available for input without replaying the whole shell frame

### F2. Windows Compatibility Fixes

- status: conditional remaining
- branch: `fix/native-platform-windows-compat`
- goal: land focused Windows-specific terminal fixes only when the validation matrix finds concrete issues
- ownership: frontend and runtime files under `src/adapter/inbound/tui/app/` only as required by validated findings
- depends on: F1, A6, and a recorded Windows validation failure
- done when:
  - each validated Windows issue is either fixed with focused regression coverage or closed with explicit manual validation notes
  - no speculative portability edits are mixed into the fix branch

## 5. Parallelism Guardrails

Keep these guardrails for the remaining slices:

- do not run A6 in parallel with another slice that changes shell rendering, transcript viewport, or prompt chrome hotspots
- do not start F2 until A6 lands and the validation matrix produces concrete Windows findings

## 6. Handoff Rule

When one remaining slice finishes:

1. rebase it onto the latest `origin/prerelease`
2. fast-forward `prerelease` to that reviewed head
3. close the PR after `prerelease` contains the commits
4. remove the finished worktree
5. update this file to move the slice into the completion snapshot before opening the next worktree
