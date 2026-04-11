# Parallel Worktree Plan

This file should stay lightweight. Expand it only while two or more live branches actually need coordination.

## Current Posture

- default base: `origin/prerelease`
- no checked-in slice plan is active right now
- if concurrent work starts, record each live slice here with branch name, goal, owned files, verification commands, and dependencies

## Hotspots

- shell flow and rendering: `src/adapter/inbound/tui/app.rs`, `src/adapter/inbound/tui/app/app_runtime.rs`, `src/adapter/inbound/tui/app/shell_rendering.rs`, `src/adapter/inbound/tui/app/shell_presentation.rs`, `src/adapter/inbound/tui/app/shell_controller.rs`
- session surfaces: `src/adapter/inbound/tui/app/session_overlay_ui.rs`, `src/adapter/inbound/tui/app/session_shell_controller.rs`
- planning surfaces: `src/adapter/inbound/tui/app/planning_init_overlay_ui.rs`, `src/adapter/inbound/tui/app/planning_draft_editor_ui.rs`, `src/application/service/planning_init_service.rs`, `src/application/service/planning_prompt_service.rs`, `src/application/service/planning_reconciliation_service.rs`
- docs and operator contract: `docs/README.md`, `docs/design/06-planning-runtime-and-draft-editor.md`, `docs/plan/10-inline-scrollback-shell.md`, `docs/plan/12-platform-validation-matrix.md`

## Usage Rule

- keep one reviewable slice and one PR per worktree
- prefer disjoint file boundaries
- if overlap is intentional, record the exact collision files here before both branches continue
- use [04-worktree-branch-rules.md](04-worktree-branch-rules.md) for the actual branch, push, and merge rules
