# Planning Init Manual Editor Rollout

This plan refines item 3 from `docs/design/06-direction-task-ledger-and-priority-queue.md`.

The goal is to land detail-mode manual authoring without collapsing multiple risky UI and filesystem changes into one branch.

Unless noted otherwise, file paths below are relative to `native/`.

## Merge Order

1. merged: `feature/native-planning-overlay-manual-draft-editor-entry`
   - delivered by PR `#128`
   - scope: `:planning` selector plus `simple mode` scaffold creation
2. current: `feature/native-planning-overlay-manual-draft-editor`
   - scope: detail-mode `manual` stages a draft editor session, loads staged files, and lets the operator edit/save/validate draft content in the shell
3. next: `feature/native-planning-service-draft-promote-flow`
   - scope: explicit `promote` action, active-path copy, and accepted planning refresh after manual editing
4. later: `feature/native-planning-overlay-llm-assisted-detail`
   - scope: future `llm-assisted` branch after the manual path is stable

## Current Slice

- branch: `feature/native-planning-overlay-manual-draft-editor`
- goal: move detail-mode `manual` from “stage and exit” to “stage and keep editing inside the shell”
- dependency: `origin/prerelease` already contains PR `#128`
- verification:
  - `cargo fmt`
  - `cargo test`

## File Ownership

- planning draft contract and persistence:
  - `src/application/port/outbound/planning_workspace_port.rs`
  - `src/adapter/outbound/filesystem_planning_workspace_adapter.rs`
  - `src/application/service/planning_init_service.rs`
- TUI editor state and rendering:
  - `src/adapter/inbound/tui/app.rs`
  - `src/adapter/inbound/tui/app/app_runtime.rs`
  - `src/adapter/inbound/tui/app/shell_controller.rs`
  - `src/adapter/inbound/tui/app/shell_presentation.rs`
  - `src/adapter/inbound/tui/app/shell_rendering.rs`
  - new planning editor UI state module under `src/adapter/inbound/tui/app/`
- regression coverage:
  - `src/adapter/inbound/tui/app/app_tests.rs`
  - `src/adapter/inbound/tui/app/shell_rendering_tests.rs`
  - service and adapter tests near the modules above

## Scope Guard

This slice should include:

- stage detail-mode draft and open a shell-native draft editor session
- load staged draft file bodies through the planning workspace port
- edit the staged draft buffers for operator-owned files
- save draft content back under `.codex-exec-loop/planning/drafts/<draft>/`
- validate the staged draft without mutating active planning files

This slice should not include:

- full active-path promote and runtime refresh if that forces additional reconciliation churn
- the future `llm-assisted` draft generation branch
- unrelated shell refactors outside the planning-init/manual path

## Expected Follow-Up

The next slice should pick up once the editor session is stable:

- explicit `promote` action from staged draft to active planning path
- post-promote planning prompt refresh and accepted planning summary
- stronger exit/cancel messaging around unsaved or invalid draft state
