# Parallel Worktree Plan

This file is the compact current-state snapshot used before a new split plan exists.

Unless noted otherwise, file paths below are relative to `native/`.

## Current Baseline
- snapshot date: `2026-04-09`
- reference branch: `origin/prerelease`
- the first native delivery pass is already integrated into `prerelease`
- no carried-over sprint checklist or completion log should be treated as live context here
- when a new sprint starts, open a dedicated feature doc and keep this file as the short concurrency snapshot

## Current Hotspots
- shell flow and rendering: `src/adapter/inbound/tui/app.rs`, `src/adapter/inbound/tui/app/app_runtime.rs`, `src/adapter/inbound/tui/app/ratatui_frontend.rs`, `src/adapter/inbound/tui/app/shell_rendering.rs`, `src/adapter/inbound/tui/app/shell_presentation.rs`, `src/adapter/inbound/tui/app/shell_controller.rs`, `src/adapter/inbound/tui/app/transcript_viewport.rs`
- inspection and follow-up surfaces: `src/adapter/inbound/tui/app/followup_overlay_ui.rs`, `src/adapter/inbound/tui/app/session_overlay_ui.rs`, and related tests under `src/adapter/inbound/tui/app/`
- shared runtime and app-server boundary: `src/adapter/inbound/tui/app/conversation_runtime.rs`, outbound app-server adapters, and request-policy docs in `design/04-hexagonal-runtime-architecture.md`
- docs and operator contract: `docs/README.md`, `docs/design/*.md`, `docs/plan/10-inline-scrollback-shell.md`, and `docs/plan/12-platform-validation-matrix.md`

## Current Worktree Posture
- keep `prerelease` in one integration checkout only
- branch new worktrees from the latest `origin/prerelease`
- keep one reviewable slice and one PR per worktree
- prefer disjoint file boundaries when two workers are active
- if overlap is intentional, record the hotspot files in the task note or PR body

## Handoff Rule
- use `04-worktree-branch-rules.md` for push, PR, and linear integration rules
- create a new feature-specific plan when the next sprint opens a concrete workstream
- do not repurpose this file into a rolling completion log or stale backlog
