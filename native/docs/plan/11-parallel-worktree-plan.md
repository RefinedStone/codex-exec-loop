# Parallel Worktree Plan

This document now records which slices from the first concurrent native delivery pass are already landed and keeps detailed planning only for the slices that still remain.

Unless noted otherwise, file paths below are relative to `native/`.

Status baseline:
- snapshot date: `2026-04-09`
- current reference branch: `origin/prerelease`
- source backlog: `docs/plan/02-todo-backlog.md`, repository root `../TODO.md`, and `docs/plan/10-inline-scrollback-shell.md`
- compaction rule: landed slices stay short here; only remaining slices keep detailed execution notes
- primary remaining product work: lane A `A4` and `A5`; the other remaining items are support or follow-up slices

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

Additional landed follow-ups that were not explicit slices in the original lane table:

- [x] startup-pending manual prompt submission queue
- [x] transcript jump shell commands
- [x] native package checksum and verification helper follow-ups

## 4. Remaining Slices

### A5. Scrollback-Safe Streaming History

- status: remaining
- branch: `feature/native-shell-stream-history`
- goal: make streaming output append into stable history without replaying the full shell frame
- ownership: `src/adapter/inbound/tui/app/conversation_runtime.rs`, `src/adapter/inbound/tui/app/shell_rendering.rs`, `src/adapter/inbound/tui/app/shell_presentation.rs`, and transcript-related tests under `src/adapter/inbound/tui/app/`
- depends on: A3
- done when:
  - terminal scrollback reads like a coherent session instead of a frame repaint log
  - buffered manual input and auto follow-up behavior remain understandable while streaming
- verify with:
  - `cargo fmt`
  - `cargo build`
  - `cargo test`

### F2. Windows Compatibility Fixes

- status: conditional remaining
- branch: `fix/native-platform-windows-compat`
- goal: land focused Windows-specific terminal fixes only when the validation matrix finds concrete issues
- ownership: frontend and runtime files under `src/adapter/inbound/tui/app/` only as required by validated findings
- depends on: F1 and a recorded Windows validation failure
- done when:
  - each validated Windows issue is either fixed with focused regression coverage or closed with explicit manual validation notes
  - no speculative portability edits are mixed into the fix branch

### M1. Root README Migration Cleanup

- status: remaining
- branch: `docs/native-readme-migration`
- goal: reduce Python CLI prominence in repository-root docs now that native is the main product path
- ownership: `../README.md` and any root-level migration notes
- done when:
  - the root README reads as a native-first product document
  - Python CLI instructions remain only as legacy or compatibility guidance

## 5. Parallelism Guardrails

Keep these guardrails for the remaining slices:

- do not run A5 in parallel with another slice that changes transcript rendering or shell presentation hotspots
- do not start F2 until the validation matrix produces concrete Windows findings
- docs-only migration cleanup can run alongside A5 because it does not touch runtime hotspots

## 6. Handoff Rule

When one remaining slice finishes:

1. rebase it onto the latest `origin/prerelease`
2. fast-forward `prerelease` to that reviewed head
3. close the PR after `prerelease` contains the commits
4. remove the finished worktree
5. update this file to move the slice into the completion snapshot before opening the next worktree
