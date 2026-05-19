# TUI Coverage Matrix

This matrix tracks automated coverage for `src/adapter/inbound/tui/**`. Use it with
[`terminal-ui-testing-methodology.md`](terminal-ui-testing-methodology.md) before adding or changing
native TUI behavior.

## Coverage Rules

- Every source file under `src/adapter/inbound/tui/**` must be mapped to a tested surface or an
  explicit architecture-test exception with a reason.
- Rendering, redraw order, host scrollback, viewport, resize, prompt, live-tail, and parallel
  event-stream changes need targeted assertions before snapshots.
- Temporal redraw regressions use the shared `tui_testkit::InlineFrameRecorder`; one-off local
  frame recorders are not allowed.
- Ratatui `TestBackend` covers deterministic screen/buffer state.
- `insta` snapshots pin stable full-frame surfaces only after targeted assertions protect the
  behavior.
- vt100-backed tests cover ANSI, cursor, clear, wrapping, and terminal scrollback behavior.

## Surface Matrix

| Surface | Source scope | Automated entry points | Current contract | Next-priority gaps |
| --- | --- | --- | --- | --- |
| Inline terminal, host scrollback, viewport, resize, redraw transaction | `app/inline_terminal_adapter/**`, `app/history_insertion.rs` | `app/inline_terminal_adapter/tests.rs`, `app/inline_terminal_adapter/tests/history_flush.rs`, `app/history_insertion.rs` | Host scrollback insertions, viewport replay, resize, redraw cache invalidation, fallback insertion, vt100 terminal behavior, and direct frame-recorder regressions. | Keep new temporal bugs in `InlineFrameRecorder`; add width-specific vt100 cases when terminal escape behavior changes. |
| Parallel event stream, live-tail, prompt position, command hints | `app/parallel_*`, `app/parallel_mode/**`, `app/shell_presentation/overlays/popup/supersession/**`, `app/shell_presentation/status_panels/**` | `app/inline_terminal_adapter/tests.rs`, `app/shell_rendering_contract_tests.rs`, `app/shell_runtime/tests/flows.rs`, `app/shell_runtime/tests/input.rs`, `app/parallel_peek_overlay_ui.rs` | Event stream rows survive redraws, panel chrome stays out of host scrollback, compact prompt remains visible, command hints stay in live UI, parallel peek state handles selection and preview navigation. | Add frame-recorder tests for any new parallel live panel or command-hint row that can move between scrollback and live viewport. |
| Overlay surfaces: help, session, planning, model/view selection, parallel peek | `app/*_overlay_ui.rs`, `app/planning/**`, `app/planning_*`, `app/session_overlay_ui.rs`, `app/shell_presentation/overlays/**`, `shell_chrome.rs` | `app/shell_rendering_contract_tests.rs`, `app/shell_rendering_contract_tests/planning.rs`, `app/shell_rendering_tests.rs`, `app/planning_draft_editor_ui/tests.rs`, `app/planning/controller.rs`, `app/session_overlay_ui.rs`, `app/model_selection_overlay_ui.rs`, `app/view_selection_overlay_ui.rs`, `shell_chrome.rs` | Popup-free inline inspections, overlay focus ownership, planning editor close guards, session browser state, model/view pickers, queue/planning snapshots with targeted assertions. | Add pure projection tests when a new overlay section builder appears before relying on a full-frame snapshot. |
| Shell runtime input flow: key events, command palette, submit, escape/cancel | `app/shell_runtime/**`, `app/conversation*`, `app/inline_shell_commands/**`, shell command modules | `app/shell_runtime/tests/input.rs`, `app/shell_runtime/tests/flows.rs`, `app/shell_runtime/tests/scheduler.rs`, `app/conversation_input.rs`, `app/conversation_intents.rs`, `app/inline_shell_commands/tests.rs` | Key dispatch, command palette, `:parallel`, `:peek`, prompt buffering, submit gates, resize redraw requests, scheduler coalescing, escape/cancel behavior. | Add input-flow tests beside any new command parser or shell key handler before changing rendering. |
| Shell rendering snapshots plus targeted assertions | `app/shell_rendering.rs`, `app/shell_rendering/**`, `app/shell_layout.rs`, `app/shell_presentation.rs`, `app/shell_presentation/**`, `app/theme.rs`, `conversation_text.rs` | `app/shell_rendering_tests.rs`, `app/shell_rendering_contract_tests.rs`, `app/shell_rendering_contract_tests/planning.rs`, `app/snapshots/**` | Ready shell, streaming shell, viewport replay, queue/planning overlays, inline inspections, narrow terminal behavior, border-free inline layout, cursor placement. | When updating a snapshot, add or update a nearby assertion that names the regression the snapshot should catch. |
| vt100 terminal path | `app/tui_testkit.rs`, `app/history_insertion.rs`, `app/inline_terminal_adapter/**`, `app/shell_rendering_tests.rs` | `app/tui_testkit.rs`, `app/history_insertion.rs`, `app/inline_terminal_adapter/tests.rs`, `app/shell_rendering_tests.rs` | ANSI/cursor/clear output, newline fallback, host scrollback rows, visible screen text, wrapping, resize, and vt100 shell snapshots. | Add vt100 tests for any new escape sequence, terminal clear path, or cursor-sensitive prompt movement. |
| Startup, session, conversation, auto-follow, planning control state | `app.rs`, `app/app_runtime.rs`, `app/auto_follow/**`, `app/auto_follow_controls.rs`, `app/conversation/**`, `app/conversation_*`, `app/github_polling/**`, `app/post_turn_continuation.rs`, `app/turn_submission_runtime/**` | `app.rs`, `app/auto_follow_controls.rs`, `app/auto_follow_overlay_ui.rs`, `app/conversation_model_tests.rs`, `app/conversation_runtime.rs`, `app/github_polling/tests.rs`, `app/turn_submission_runtime.rs`, `app/shell_entrypoint.rs` | App state transitions, runtime effects, conversation lifecycle, auto-follow controls, GitHub polling state, turn submission, manual prompt preparation. | Add reducer tests before introducing new runtime state that can affect render output. |
| TUI support and validation devices | `app/tui_testkit.rs`, `app/test_helpers.rs`, `tests/architecture_boundaries.rs`, `tests/native_validation_scripts.rs` | `app/tui_testkit.rs`, `tests/architecture_boundaries.rs`, `tests/native_validation_scripts.rs`, this matrix | Shared render helpers, Ratatui backends, vt100 backend, frame recorder, native validation script contracts, static coverage guards. | Extend shared helpers here first when multiple TUI tests need the same device. |

## Explicit Exceptions

The architecture guard owns the source allowlist. Exceptions must stay narrow and documented there.
At the time of this matrix, only module-declaration glue may be exempted directly; test fixtures,
snapshots, and `tui_testkit` are treated as test-support paths rather than production coverage gaps.

## Validation

Run these for TUI coverage changes:

```bash
. "$HOME/.cargo/env"
cargo fmt --all -- --check
cargo test --test architecture_boundaries
cargo test inline_terminal_adapter::tests
cargo test shell_rendering_contract_tests
cargo test shell_rendering_tests
cargo test shell_runtime::tests
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```
