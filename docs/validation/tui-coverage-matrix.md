# TUI Coverage Matrix

This matrix is the executable inventory for the native Ratatui/Crossterm shell. The detailed
method is in [`terminal-ui-testing-methodology.md`](./terminal-ui-testing-methodology.md). The
first-class default is a Grok-style alternate-screen fullscreen app; there is no host-scrollback
rendering branch.

The owned render boundary is `FullscreenShellFrameModel` → `FullscreenInspectionFrameModel` →
`FullscreenFrameRenderReceipt`. Ratatui `TestBackend`, alternate-screen lifecycle tests, and the
architecture-test exception ledger cover different failure classes and must stay joined.

## Surface Coverage

| Surface | Production source | Automated proof | Contract |
|---|---|---|---|
| Unified Work Center | `app/work_center_overlay_ui.rs` | colocated reducer tests | Read-only aggregate; no shadow authority. |
| Fullscreen terminal, transcript viewport, resize, redraw transaction | `app/fullscreen_terminal_adapter.rs`, `app/fullscreen_frame_model.rs`, `app/transcript_viewport_ui.rs`, `app/ratatui_frontend.rs` | fullscreen frame, viewport, lifecycle, and rendering regressions | One app-owned viewport; stable resize-gated receipt. |
| Parallel event stream, live-tail, prompt position, command hints | `app/parallel_*`, supersession presentation | parallel stream, supervisor, peek, and fullscreen regressions | One canonical event window rendered entirely inside Ratatui. |
| Overlay surfaces: help, session, planning, model/view/language selection, parallel peek, progressive activity | overlay and presentation modules | colocated projection/reducer tests plus fullscreen rendering | Overlays consume owned immutable screen models. |
| Shell runtime input flow: key events, command palette, submit, escape/cancel | conversation input, shell controller, command modules | reducer/controller tests plus fullscreen interaction tests | PageUp/PageDown, Ctrl+Home/End, Ctrl+E, mouse and composer focus have one owner. |
| Shell rendering snapshots plus targeted assertions | `app/fullscreen_frame_model.rs`, `app/shell_rendering/**`, presentation | fullscreen rendering, layout, inspection, frame-model tests | Pure draw from an owned frame; no application or terminal I/O. |
| Alternate-screen terminal lifecycle and TestBackend path | `app/ratatui_frontend.rs`, `app/fullscreen_terminal_adapter.rs` | escape-sequence lifecycle and TestBackend delivery tests | Enter/leave alternate screen, mouse/focus/paste modes, cursor restoration. |
| Startup, session, conversation, auto-follow, planning control state | app/controller/runtime modules | colocated state and event tests | Core/application authority is projected, never recreated in the TUI. |
| TUI support and validation devices | test helpers, fullscreen regression harness, architecture guard | deterministic TestBackend and static guards | Shared helpers remain presentation-only and deterministic. |

## Fullscreen Proof Matrix

| Invariant | Deterministic proof | Manual proof trigger |
|---|---|---|
| F1 | One canonical transcript ordering | app-server event ordering changes |
| F2 | Reader anchor survives streaming append | wrapping or terminal-width behavior changes |
| F3 | Stable resize-gated receipt | terminal resize primitive changes |
| F4 | Thread switch resets viewport identity | session lifecycle changes |
| F5 | Parallel stream stays app-owned | focused parallel layout changes |
| F6 | Alternate-screen lifecycle restores the terminal | Crossterm mode or escape-sequence changes |

## Environment Applicability

- **E1** = Windows Terminal + WSL bash + fullscreen
- **E2** = Windows Terminal + PowerShell + fullscreen
- **E3** = tmux detached PTY + fullscreen
- **E4** = direct Linux terminal + fullscreen

Environment keys: `E1`, `E2`, `E3`, `E4`.

All four environments are required when alternate-screen escape sequences, viewport mode, clear or restore behavior,
mouse capture behavior, or width/wrapping primitives change. A representative
pair is sufficient for copy-only or pure projection changes. This keeps the fullscreen stack as the default posture
while controlling future test-growth cost and maintainability cost.

## Joined Proof Shape

Each meaningful TUI change records: invariant, Current owner / source, Decision point,
First-class default, Proof obligation, automated result, and any primitive-sensitive capture.
Reviewers assess bug-class recurrence across terminal boundaries.
Manual terminal capture stays primitive-sensitive only.

## Test Entry Point Inventory

- `src/adapter/inbound/tui/app.rs`
- `src/adapter/inbound/tui/app/app_runtime.rs`
- `src/adapter/inbound/tui/app/auto_follow_controls.rs`
- `src/adapter/inbound/tui/app/auto_follow_overlay_ui.rs`
- `src/adapter/inbound/tui/app/conversation_input.rs`
- `src/adapter/inbound/tui/app/conversation_intents.rs`
- `src/adapter/inbound/tui/app/conversation_lifecycle.rs`
- `src/adapter/inbound/tui/app/conversation_model/activity_rail.rs`
- `src/adapter/inbound/tui/app/conversation_model/auto_follow.rs`
- `src/adapter/inbound/tui/app/conversation_model/auto_follow_decision.rs`
- `src/adapter/inbound/tui/app/conversation_model/progressive_activity_cards.rs`
- `src/adapter/inbound/tui/app/conversation_model/progressive_activity_detail_tests.rs`
- `src/adapter/inbound/tui/app/conversation_model/progressive_activity_tests.rs`
- `src/adapter/inbound/tui/app/conversation_model/view_model.rs`
- `src/adapter/inbound/tui/app/conversation_model/view_model/messages.rs`
- `src/adapter/inbound/tui/app/conversation_runtime.rs`
- `src/adapter/inbound/tui/app/directions_maintenance_ui.rs`
- `src/adapter/inbound/tui/app/fullscreen_frame_model.rs`
- `src/adapter/inbound/tui/app/fullscreen_rendering_tests.rs`
- `src/adapter/inbound/tui/app/fullscreen_terminal_adapter.rs`
- `src/adapter/inbound/tui/app/github_polling/tests.rs`
- `src/adapter/inbound/tui/app/inline_shell_commands/tests.rs`
- `src/adapter/inbound/tui/app/language.rs`
- `src/adapter/inbound/tui/app/model_selection_overlay_ui.rs`
- `src/adapter/inbound/tui/app/parallel_mode.rs`
- `src/adapter/inbound/tui/app/parallel_mode/panel_controller.rs`
- `src/adapter/inbound/tui/app/parallel_mode/presentation_bridge.rs`
- `src/adapter/inbound/tui/app/parallel_mode_shell_command.rs`
- `src/adapter/inbound/tui/app/parallel_peek_overlay_ui.rs`
- `src/adapter/inbound/tui/app/parallel_stream_view.rs`
- `src/adapter/inbound/tui/app/parallel_supervisor_events.rs`
- `src/adapter/inbound/tui/app/planning/controller.rs`
- `src/adapter/inbound/tui/app/planning/presentation.rs`
- `src/adapter/inbound/tui/app/planning/status_projection.rs`
- `src/adapter/inbound/tui/app/planning_draft_editor_ui/tests.rs`
- `src/adapter/inbound/tui/app/planning_init_overlay_ui.rs`
- `src/adapter/inbound/tui/app/planning_overlay_shell_command.rs`
- `src/adapter/inbound/tui/app/planning_reset_shell_command.rs`
- `src/adapter/inbound/tui/app/planning_runtime_refresh_ui.rs`
- `src/adapter/inbound/tui/app/planning_shell_command.rs`
- `src/adapter/inbound/tui/app/planning_worker_debug_preview.rs`
- `src/adapter/inbound/tui/app/planning_workspace_operation_ui.rs`
- `src/adapter/inbound/tui/app/progressive_activity_overlay_ui.rs`
- `src/adapter/inbound/tui/app/queue_overlay_ui.rs`
- `src/adapter/inbound/tui/app/ratatui_frontend.rs`
- `src/adapter/inbound/tui/app/reviews_overlay_ui.rs`
- `src/adapter/inbound/tui/app/session_overlay_screen_model.rs`
- `src/adapter/inbound/tui/app/session_overlay_ui.rs`
- `src/adapter/inbound/tui/app/session_shell_controller.rs`
- `src/adapter/inbound/tui/app/shell_controller.rs`
- `src/adapter/inbound/tui/app/shell_entrypoint.rs`
- `src/adapter/inbound/tui/app/shell_frontend.rs`
- `src/adapter/inbound/tui/app/shell_layout.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/activity.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/activity_diff/tests.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/directions.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/parallel_peek.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/planning_editor_inputs.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/planning_existing_workspace.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/planning_existing_workspace_inputs.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/planning_init_copy.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/planning_runtime.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/planning_session.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/queue.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/supersession/tests.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/work_center.rs`
- `src/adapter/inbound/tui/app/shell_presentation/prompt_composer.rs`
- `src/adapter/inbound/tui/app/shell_presentation/runtime_status_copy.rs`
- `src/adapter/inbound/tui/app/shell_presentation/session_browser.rs`
- `src/adapter/inbound/tui/app/shell_presentation/session_browser/empty_state.rs`
- `src/adapter/inbound/tui/app/shell_presentation/startup_banner.rs`
- `src/adapter/inbound/tui/app/shell_presentation/status_panels/activity_rail.rs`
- `src/adapter/inbound/tui/app/shell_presentation/status_panels/operator_ribbon.rs`
- `src/adapter/inbound/tui/app/shell_presentation/status_panels/parallel_working_copy.rs`
- `src/adapter/inbound/tui/app/shell_presentation/status_panels/plan_indicator.rs`
- `src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_shared.rs`
- `src/adapter/inbound/tui/app/shell_presentation/terminal_text.rs`
- `src/adapter/inbound/tui/app/shell_presentation/transcript_copy.rs`
- `src/adapter/inbound/tui/app/shell_rendering/fullscreen_inspection.rs`
- `src/adapter/inbound/tui/app/shell_rendering/fullscreen_layout.rs`
- `src/adapter/inbound/tui/app/shell_runtime.rs`
- `src/adapter/inbound/tui/app/test_helpers.rs`
- `src/adapter/inbound/tui/app/transcript_viewport_ui.rs`
- `src/adapter/inbound/tui/app/turn_submission_runtime.rs`
- `src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution.rs`
- `src/adapter/inbound/tui/app/view_selection_overlay_ui.rs`
- `src/adapter/inbound/tui/app/work_center_overlay_ui.rs`
- `src/adapter/inbound/tui/conversation_text.rs`
- `src/adapter/inbound/tui/shell_chrome.rs`
- `src/adapter/inbound/tui/supersession_mud.rs`

## Explicit Exceptions

The architecture guard owns the source allowlist. An architecture-test exception must stay narrow,
name its owner, and explain why no executable behavior exists in that file.

## Validation

```bash
cargo fmt --all -- --check
cargo test --lib -- --test-threads=1
cargo test --test architecture_boundaries -- --test-threads=1
cargo clippy --all-targets --all-features -- -D warnings
```
