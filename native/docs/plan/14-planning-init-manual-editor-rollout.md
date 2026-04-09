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
3. merged: `feature/native-planning-service-draft-promote-flow`
   - delivered by PR `#130`
   - scope: explicit `promote` action, active-path copy, and accepted planning refresh after manual editing
4. current: `feature/native-planning-overlay-editor-exit-guards`
   - scope: stronger exit/cancel messaging when the draft editor has unsaved or invalid state
5. later: `feature/native-planning-overlay-llm-assisted-detail`
   - scope: future `llm-assisted` branch after the manual path is stable
6. later: `feature/native-planning-overlay-post-promote-summary`
   - scope: richer operator summary after promote and any follow-on reconciliation status

## Current Slice

- branch: `feature/native-planning-overlay-editor-exit-guards`
- goal: keep the manual editor from disappearing abruptly when unsaved edits or an invalid staged draft would make close ambiguous
- dependency: `origin/prerelease` already contains PR `#130`
- verification:
  - `cargo fmt`
  - `cargo test`

## File Ownership

- TUI editor state and rendering:
  - `src/adapter/inbound/tui/app.rs`
  - `src/adapter/inbound/tui/app/planning_draft_editor_ui.rs`
  - `src/adapter/inbound/tui/app/shell_controller.rs`
  - `src/adapter/inbound/tui/app/shell_presentation.rs`
  - `src/adapter/inbound/tui/app/shell_rendering.rs`
- regression coverage:
  - `src/adapter/inbound/tui/app/app_tests.rs`
  - `src/adapter/inbound/tui/app/shell_rendering_tests.rs`
  - `src/adapter/inbound/tui/app/planning_draft_editor_ui.rs`

## Scope Guard

This slice should include:

- explicit close confirmation when unsaved in-memory edits would be discarded
- explicit close confirmation when the last saved staged draft is invalid
- clear operator messaging for confirm close vs continue editing
- regression coverage for close confirmation and clean close behavior

This slice should not include:

- the future `llm-assisted` draft generation branch
- richer post-promote summary or reconciliation drill-down
- unrelated shell refactors outside the planning-init/manual path

## Expected Follow-Up

The next slice should pick up once the editor session is stable:

- richer post-promote operator summary if accepted planning needs additional runtime reconciliation
