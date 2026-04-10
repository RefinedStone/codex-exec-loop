# Parallel Worktree Plan

Unless noted otherwise, file paths below are relative to the repository root.

## Current Posture
- reference branch: `origin/prerelease`
- active feature split: `docs/plan/14-planning-init-manual-editor-rollout.md`
- active planning-runtime follow-up: `docs/plan/15-planning-runtime-engagement-repair.md`
- active app-cluster slimming slice: `fix/native-session-shell-controller-review-followup`
- treat the docs above as the live slice plans for current planning work

## Current Hotspots
- shell flow and rendering: `src/adapter/inbound/tui/app.rs`, `src/adapter/inbound/tui/app/app_runtime.rs`, `src/adapter/inbound/tui/app/ratatui_frontend.rs`, `src/adapter/inbound/tui/app/shell_rendering.rs`, `src/adapter/inbound/tui/app/shell_presentation.rs`, `src/adapter/inbound/tui/app/shell_controller.rs`, `src/adapter/inbound/tui/app/transcript_viewport.rs`
- inspection and follow-up surfaces: `src/adapter/inbound/tui/app/followup_overlay_ui.rs`, `src/adapter/inbound/tui/app/session_overlay_ui.rs`, and related tests under `src/adapter/inbound/tui/app/`
- shared runtime and app-server boundary: `src/adapter/inbound/tui/app/conversation_runtime.rs`, outbound app-server adapters, and request-policy docs in `design/04-hexagonal-runtime-architecture.md`
- planning draft authoring boundary: `src/application/port/outbound/planning_workspace_port.rs`, `src/adapter/outbound/filesystem_planning_workspace_adapter.rs`, `src/application/service/planning_init_service.rs`, and planning-init shell overlays
- planning shell presentation boundary: `src/adapter/inbound/tui/app/shell_presentation.rs`, `src/adapter/inbound/tui/app/planning_presentation.rs`, and follow-up preview/status tests under `src/adapter/inbound/tui/app/`
- planning turn submission boundary: `src/adapter/inbound/tui/app/app_runtime.rs`, `src/adapter/inbound/tui/app/turn_submission_runtime.rs`, and prompt/follow-up runtime tests under `src/adapter/inbound/tui/app/`
- session shell controller boundary: `src/adapter/inbound/tui/app/shell_controller.rs`, `src/adapter/inbound/tui/app/session_shell_controller.rs`, and session overlay tests under `src/adapter/inbound/tui/app/`
- docs and operator contract: `docs/README.md`, `docs/design/*.md`, `docs/plan/10-inline-scrollback-shell.md`, and `docs/plan/12-platform-validation-matrix.md`

## Worktree Posture
- keep `prerelease` in one integration checkout only
- branch new worktrees from the latest `origin/prerelease`
- keep one reviewable slice and one PR per worktree
- prefer disjoint file boundaries when two workers are active
- if overlap is intentional, record the hotspot files in the task note or PR body

## Reset Rule
- use `04-worktree-branch-rules.md` for push, PR, and linear integration rules
- once the planning-init/manual-editor rollout closes, this file can return to the lightweight placeholder posture
