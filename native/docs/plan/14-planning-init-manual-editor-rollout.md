# Planning Init Manual Editor Rollout

This plan refines item 3 from `docs/design/06-direction-task-ledger-and-priority-queue.md`.

The goal is to land detail-mode manual authoring without collapsing multiple risky UI and filesystem changes into one branch.

Unless noted otherwise, file paths below are relative to `native/`.

## Merge Order

1. merged: `feature/native-planning-overlay-manual-draft-editor-entry`
   - delivered by PR `#128`
   - scope: `:planning` selector plus `simple mode` scaffold creation
2. merged: `feature/native-planning-overlay-manual-draft-editor`
   - delivered by PR `#129`
   - scope: detail-mode `manual` stages a draft editor session, loads staged files, and lets the operator edit/save/validate draft content in the shell
3. current: `feature/native-planning-service-draft-promote-flow`
   - scope: explicit `promote` action, active-path copy, and accepted planning refresh after manual editing
4. later: `feature/native-planning-overlay-llm-assisted-detail`
   - scope: future `llm-assisted` branch after the manual path is stable

## Current Slice

- branch: `feature/native-planning-service-draft-promote-flow`
- goal: add explicit promote from the embedded draft editor into the active planning path and refresh the accepted planning context in the shell
- dependency: `origin/prerelease` already contains PR `#129`
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

- explicit promote action from the embedded draft editor
- re-use staged draft validation before mutating active planning files
- copy accepted draft content into `.codex-exec-loop/planning/`
- refresh the ready conversation's planning prompt context after promote

This slice should not include:

- the future `llm-assisted` draft generation branch
- unrelated shell refactors outside the planning-init/manual path

## Expected Follow-Up

The next slice should pick up once the editor session is stable:

- stronger exit/cancel messaging around unsaved or invalid draft state
- richer post-promote operator summary if accepted planning needs additional runtime reconciliation
