# TUI Coverage Matrix

[한국어 통합 안내](../ko/reference/validation.md)

This matrix tracks automated coverage for `src/adapter/inbound/tui/**`. Use it with
[`terminal-ui-testing-methodology.md`](terminal-ui-testing-methodology.md) before adding or changing
native TUI behavior.

## Coverage Rules

- Every source file under `src/adapter/inbound/tui/**` must be mapped to a tested surface or an
  explicit architecture-test exception with a reason.
- Rendering, redraw order, host scrollback, viewport, resize, prompt, live-tail, and parallel
  event-stream changes need targeted assertions before snapshots.
- Inline inspection renderers must route through the typed render surface API; generic low-level
  titled/scrolled helpers stay private to the layout module.
- Review Center presentation must consume its request-correlated immutable screen model; repeated
  redraws must perform no application, repository, filesystem, or database I/O.
- Native TUI startup must keep prompt-log storage maintenance outside app construction and the
  first-frame path. Its regression proof must gate the maintenance port for 650 ms while
  `prepare_runtime` and first-draw eligibility remain non-blocking.
- Temporal redraw regressions use the shared `tui_testkit::InlineFrameRecorder`; one-off local
  frame recorders are not allowed.
- Ratatui `TestBackend` covers deterministic screen/buffer state.
- `insta` snapshots pin stable full-frame surfaces only after targeted assertions protect the
  behavior.
- vt100-backed tests cover ANSI, cursor, clear, wrapping, and terminal scrollback behavior.
## Proof Contract Markers

Option A remains the current-stack default on the existing Ratatui/Crossterm structure. This
matrix materializes the release-blocking proof shape for that stack; it does not by itself
activate Option B.
The current stack as the default posture remains explicit in this matrix for repo-facing proof.

### First-class environment key

- **E1** = Windows Terminal + WSL bash + inline
- **E2** = Windows Terminal + PowerShell + inline
- **E3** = tmux detached PTY + inline
- **E4** = direct Linux terminal + inline

### Branch-family key

- **B1** = `HostScrollback`
- **B2** = `ViewportReplay`
- **B3** = `StandardScrollRegion`
- **B4** = `NewlineFallback`
- Branch-family keys: `HostScrollback`, `ViewportReplay`, `StandardScrollRegion`, `NewlineFallback`.

## Primary Proof Matrix — Invariant × First-Class Environment

| Invariant ID | Invariant | E1 | E2 | E3 | E4 | Automated proof layer | Manual capture required? |
| --- | --- | --- | --- | --- | --- | --- | --- |
| I1 | History/live-tail separation and no duplicate replay | release-blocking | release-blocking | release-blocking | release-blocking | direct frame recorder + `TestBackend`; vt100 if primitive path changed | yes if scrollback/viewport/insertion primitive behavior changed |
| I2 | Resize leaves no stale rows or duplicated live tail | release-blocking | release-blocking | release-blocking | release-blocking | direct frame recorder + vt100/backend resize + scheduler/runtime tests | yes if resize/viewport primitive behavior changed |
| I3 | Scrollback insertion / clear-reset restores clean header and viewport | release-blocking | release-blocking | release-blocking | release-blocking | `TestBackend` + vt100 + frame recorder for temporal cases | yes if clear/reset/scrollback primitive behavior changed |
| I4 | Thread/session switch does not leak transcript or deferred history | release-blocking | release-blocking | release-blocking | release-blocking | reducer/runtime tests; frame recorder if redraw-order leakage is involved | no by default; yes only if primitive reset/viewport behavior changed |
| I5 | `ViewportReplay` remains explicit-only and does not write committed history to host scrollback | representative | representative | representative | representative | `TestBackend` + frame recorder | yes only if viewport primitive behavior changed |
| I6 | Standard and fallback insertion modes each preserve viewport state correctly | release-blocking where default or explicitly claimed; representative otherwise | release-blocking where default or explicitly claimed; representative otherwise | release-blocking where default or explicitly claimed; representative otherwise | release-blocking where default or explicitly claimed; representative otherwise | `TestBackend` + vt100 | yes if insertion strategy or escape-sequence behavior changed |

## Linked Branch-Family Applicability Table — Invariant × Branch Family

| Invariant ID | B1 HostScrollback | B2 ViewportReplay | B3 StandardScrollRegion | B4 NewlineFallback | Target test family / entrypoint |
| --- | --- | --- | --- | --- | --- |
| I1 | primary required | representative required | applicable where history flush uses standard insertion | applicable where history flush uses fallback insertion | `src/adapter/inbound/tui/app/inline_terminal_adapter/tests.rs`: `host_history_sync_keeps_live_agent_delta_out_of_inserted_history`, `host_scrollback_preserves_multiturn_history_beyond_screen_cap_without_duplicates`, `parallel_stream_preserves_initial_status_rows_as_runtime_events_advance` |
| I2 | required | required | required when resize intersects standard insertion path | required when resize intersects fallback insertion path | `inline_terminal_adapter/tests.rs`: `viewport_replay_resize_does_not_push_tail_into_scrollback`, `host_scrollback_resize_does_not_push_tail_into_scrollback`, `draw_internal_resize_does_not_push_tail_into_scrollback`, `vt100_terminal_app_preserves_newline_fallback_history_after_live_resize`; `shell_runtime/tests/scheduler.rs` |
| I3 | applicable when committed history writes to host scrollback | applicable if clear/reset semantics diverge in replay path | primary required | primary required where default/claimed | `history_insertion.rs` tests: `standard_scroll_region_inserts_history_before_inline_viewport`, `newline_fallback_inserts_history_without_scroll_regions`, cursor restore tests; `inline_terminal_adapter/tests.rs` back-buffer invalidation tests |
| I4 | applicable after switch if host history flush exists | applicable after switch if replay viewport state exists | branch relevance only if reset path touches standard insertion | branch relevance only if reset path touches fallback insertion | `shell_runtime/tests/input.rs`, flow tests, conversation/runtime entrypoints listed in this matrix |
| I5 | not primary | primary required | not primary | not primary | `inline_terminal_adapter/tests.rs`: `viewport_replay_sync_skips_host_scrollback_insertions`, `viewport_replay_keeps_inline_viewport_for_shell_positioning` |
| I6 | branch relevance only when host history uses insertion strategy | branch relevance only if replay path still exercises insertion semantics | primary required | primary required | `history_insertion.rs` tests for standard/newline fallback, wide-char, wrapped suffix, cursor restore; vt100 fallback regressions in `inline_terminal_adapter/tests.rs` |

## Joined Proof Shape

Read the durable proof contract as a join:

1. choose an **Invariant ID** from the primary environment matrix
2. read its obligation across **E1-E4**
3. join that same **Invariant ID** to the linked branch-family table for **B1-B4** applicability
   and target entrypoints

This keeps **invariant × first-class environment × branch family** explicit instead of leaving the
environment or branch dimension in prose.

## Decision Record Schema Expectation

- Mandatory Decision Record axes: `bug-class recurrence across compatibility boundaries`, `fallback masking risk`, `future test-growth cost`, `maintainability cost`.
- Compatibility-tier ownership table shape: `Current owner / source`, `Decision point`, `First-class default`, `Fallback / experimental handling`, `Override mechanism`, `Downgrade semantics`, `Proof obligation`.
- Manual terminal capture stays primitive-sensitive only and is not automatic Option B activation.
- Primitive-sensitive trigger wording: `escape sequences`, `viewport mode`, `clear or restore behavior`, `host scrollback behavior`.
## Surface Matrix

| Surface | Source scope | Automated entry points | Current contract | Next-priority gaps |
| --- | --- | --- | --- | --- |
| Inline terminal, host scrollback, viewport, resize, redraw transaction | `app/inline_terminal_adapter/**`, `app/history_insertion.rs` | `app/inline_terminal_adapter/tests.rs`, `app/inline_terminal_adapter/tests/history_flush.rs`, `app/history_insertion.rs` | Host scrollback insertions, viewport replay, resize, focus-reacquire visible-buffer rebuild, redraw cache invalidation, fallback insertion, vt100 terminal behavior, and direct frame-recorder regressions. | Keep new temporal bugs in `InlineFrameRecorder`; add width-specific vt100 cases when terminal escape behavior changes. |
| Parallel event stream, live-tail, prompt position, command hints | `app/parallel_*`, `app/parallel_mode/**`, `app/shell_presentation/overlays/popup/supersession/**`, `app/shell_presentation/status_panels/**` | `app/inline_terminal_adapter/tests.rs`, `app/shell_rendering_contract_tests.rs`, `app/shell_runtime/tests/flows.rs`, `app/shell_runtime/tests/input.rs`, `app/parallel_peek_overlay_ui.rs` | Event stream rows survive redraws, split scrollback/live-tail streams render as a titleless live tail, panel chrome stays out of host scrollback, compact prompt remains visible, command hints stay in live UI, parallel peek state handles selection and preview navigation. | Add frame-recorder tests for any new parallel live panel, title visibility, stream offset, or command-hint row that can move between scrollback and live viewport. |
| Overlay surfaces: help, session, planning, model/view/language selection, parallel peek, progressive activity | `app/language.rs`, `app/*_overlay_ui.rs`, `app/directions_maintenance_ui.rs`, `app/progressive_activity_overlay_ui.rs`, `app/queue_overlay_controller.rs`, `app/queue_overlay_ui.rs`, `app/planning/**`, `app/planning_*`, `app/session_overlay_ui.rs`, `app/shell_presentation/overlays/**`, `app/shell_presentation/overlays/activity.rs`, `shell_chrome.rs` | `app/directions_maintenance_ui.rs`, `app/inline_terminal_adapter/tests.rs`, `app/shell_controller.rs`, `app/shell_presentation/overlays/activity.rs`, `app/shell_presentation/overlays/activity_diff/tests.rs`, `app/shell_rendering_contract_tests.rs`, `app/shell_rendering_contract_tests/planning.rs`, `app/shell_rendering_tests.rs`, `app/language.rs`, `app/planning_draft_editor_ui/tests.rs`, `app/planning/controller.rs`, `app/planning_workspace_operation_ui.rs`, `app/progressive_activity_overlay_ui.rs`, `app/queue_overlay_ui.rs`, `app/reviews_overlay_ui.rs`, `app/session_overlay_ui.rs`, `app/model_selection_overlay_ui.rs`, `app/view_selection_overlay_ui.rs`, `shell_chrome.rs` | Popup-free inline inspections, overlay focus ownership, planning editor close guards, session browser state, model/view/language pickers, queue selection and cancellation, conversation-tail queue receipt undo, queue/planning snapshots, and bounded Diff/Output pages with resize, approval-priority, ANSI, and host-scrollback assertions. Review Center, Planning Queue, and Directions Maintenance authority load into request-correlated screen snapshots; presentation consumes those immutable models; repeated redraws perform no application or repository I/O. Directions loading additionally proves non-blocking open, latest-request correlation, workspace-drift rejection, and late-completion rejection after close. The shared planning-workspace coordinator proves reset, simple stage/load/promotion, planning/directions editor staging, and all four editor save/promote entry points use non-blocking dispatch, full target/session/buffer-revision identity, exact duplicate coalescing, busy exclusion, zero provider I/O for malformed identity, one existing action use case, redacted panic completion, stale/duplicate/ABA/wrong-kind rejection, close/workspace/approval/new-session drift rejection, coalesced presentation-revision rebinding, in-flight editing, A→B→A promotion refresh, workspace-B no-refresh, save without pause/refresh, and exact promotion authority refresh without replacing newer status, body, dirty state, or overlay intent. Rust-aware AST guards reject every production TUI direct save/promote callable form and `.workspace()` exposure. | Add pure projection tests when a new overlay section builder appears before relying on a full-frame snapshot; keep real-terminal capture with the primitive-sensitive validation lane. |
| Shell runtime input flow: key events, command palette, submit, escape/cancel | `app/shell_runtime/**`, `app/conversation*`, `app/inline_shell_commands/**`, shell command modules | `app/shell_runtime/tests.rs`, `app/shell_runtime/tests/input.rs`, `app/shell_runtime/tests/flows.rs`, `app/shell_runtime/tests/scheduler.rs`, `app/conversation_input.rs`, `app/conversation_intents.rs`, `app/inline_shell_commands/tests.rs` | Key and mouse dispatch, queue receipt click cancellation, command palette, `:parallel`, `:peek`, prompt buffering, submit gates, resize redraw requests, scheduler coalescing, escape/cancel behavior. | Add input-flow tests beside any new command parser or shell input handler before changing rendering. |
| Shell rendering snapshots plus targeted assertions | `app/shell_rendering.rs`, `app/shell_rendering/**`, `app/shell_layout.rs`, `app/shell_presentation.rs`, `app/shell_presentation/**`, `app/theme.rs`, `conversation_text.rs` | `app/shell_rendering_tests.rs`, `app/shell_rendering_contract_tests.rs`, `app/shell_rendering_contract_tests/planning.rs`, `app/snapshots/**` | Ready shell, streaming shell, viewport replay, queue/planning overlays, inline inspections, queue undo mouse hit targets, typed render surface routing, narrow terminal behavior, border-free inline layout, cursor placement, one immutable conversation tail projection, and modal cursor hide/exact restore. | When updating a snapshot, add or update a nearby assertion that names the regression the snapshot should catch. |
| vt100 terminal path | `app/tui_testkit.rs`, `app/history_insertion.rs`, `app/inline_terminal_adapter/**`, `app/shell_rendering_tests.rs` | `app/tui_testkit.rs`, `app/history_insertion.rs`, `app/inline_terminal_adapter/tests.rs`, `app/shell_rendering_tests.rs` | ANSI/cursor/clear output, newline fallback, host scrollback rows, visible screen text, wrapping, resize, and vt100 shell snapshots. | Add vt100 tests for any new escape sequence, terminal clear path, or cursor-sensitive prompt movement. |
| Startup, session, conversation, auto-follow, planning control state | `app.rs`, `app/app_runtime.rs`, `app/auto_follow/**`, `app/auto_follow_controls.rs`, `app/conversation/**`, `app/conversation_*`, `app/github_polling/**`, `app/post_turn_continuation.rs`, `app/turn_submission_runtime/**` | `app.rs`, `app/app_runtime.rs`, `app/auto_follow_controls.rs`, `app/auto_follow_overlay_ui.rs`, `app/conversation_model_tests.rs`, `app/conversation_runtime.rs`, `app/github_polling/tests.rs`, `app/inline_terminal_adapter/tests.rs`, `app/shell_runtime/tests.rs`, `app/turn_submission_runtime.rs`, `app/shell_entrypoint.rs`; Core controller/effect-runner tests and `tests/architecture_boundaries.rs` | App state transitions, runtime effects, conversation lifecycle, auto-follow controls, turn submission, and manual prompt preparation. Startup proves a 650 ms gated prompt-log maintenance port cannot block build/prepare or first-draw eligibility; exact generation/workspace correlation rejects stale A→B→A, mismatched, duplicate, and redacted panic completions. GitHub setup proves zero dispatch on failed draw, one dispatch after the first stable delivered frame, a sub-300ms first frame while a 650ms provider is blocked, live input/redraw during setup, exact generation/workspace service binding, A→B→A/stale/duplicate/panic fail-closed behavior, and setup completion before polling. Rust-aware guards exclude comments, strings, and test-only items while banning direct prompt-log maintenance and GitHub provider/process ownership in production TUI paths. | Add reducer tests before introducing new runtime state that can affect render output; preserve the first-frame timing and exact-correlation setup proofs when changing bootstrap or terminal delivery. |
| TUI support and validation devices | `app/tui_testkit.rs`, `app/test_helpers.rs`, `tests/architecture_boundaries.rs`, `tests/native_validation_scripts.rs` | `app/tui_testkit.rs`, `tests/architecture_boundaries.rs`, `tests/native_validation_scripts.rs`, this matrix | Shared render helpers, Ratatui backends, vt100 backend, frame recorder, native validation script contracts, static coverage guards. | Extend shared helpers here first when multiple TUI tests need the same device. |

## Test Entry Point Inventory

`tests/architecture_boundaries.rs` reads this section and compares it with the
actual Rust files under `src/adapter/inbound/tui/**` that contain TUI tests or
snapshot assertions. Add a path here whenever a new TUI test entrypoint is added,
and remove it when the corresponding tests move or disappear.

- `src/adapter/inbound/tui/app.rs`
- `src/adapter/inbound/tui/app/app_runtime.rs`
- `src/adapter/inbound/tui/app/auto_follow_controls.rs`
- `src/adapter/inbound/tui/app/auto_follow_overlay_ui.rs`
- `src/adapter/inbound/tui/app/conversation_input.rs`
- `src/adapter/inbound/tui/app/conversation_intents.rs`
- `src/adapter/inbound/tui/app/conversation_lifecycle.rs`
- `src/adapter/inbound/tui/app/conversation_model/activity_rail.rs`
- `src/adapter/inbound/tui/app/conversation_model/auto_follow_decision.rs`
- `src/adapter/inbound/tui/app/conversation_model/progressive_activity_cards.rs`
- `src/adapter/inbound/tui/app/conversation_model/progressive_activity_detail_tests.rs`
- `src/adapter/inbound/tui/app/conversation_model/progressive_activity_tests.rs`
- `src/adapter/inbound/tui/app/conversation_model/view_model.rs`
- `src/adapter/inbound/tui/app/conversation_model/view_model/messages.rs`
- `src/adapter/inbound/tui/app/conversation_model_tests.rs`
- `src/adapter/inbound/tui/app/conversation_runtime.rs`
- `src/adapter/inbound/tui/app/directions_maintenance_ui.rs`
- `src/adapter/inbound/tui/app/github_polling/tests.rs`
- `src/adapter/inbound/tui/app/history_insertion.rs`
- `src/adapter/inbound/tui/app/inline_shell_commands/tests.rs`
- `src/adapter/inbound/tui/app/inline_terminal_adapter/tests.rs`
- `src/adapter/inbound/tui/app/inline_terminal_adapter/tests/history_flush.rs`
- `src/adapter/inbound/tui/app/language.rs`
- `src/adapter/inbound/tui/app/model_selection_overlay_ui.rs`
- `src/adapter/inbound/tui/app/parallel_mode.rs`
- `src/adapter/inbound/tui/app/parallel_mode/panel_controller.rs`
- `src/adapter/inbound/tui/app/parallel_mode/presentation_bridge.rs`
- `src/adapter/inbound/tui/app/parallel_mode_shell_command.rs`
- `src/adapter/inbound/tui/app/parallel_peek_overlay_ui.rs`
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
- `src/adapter/inbound/tui/app/reviews_overlay_ui.rs`
- `src/adapter/inbound/tui/app/session_overlay_ui.rs`
- `src/adapter/inbound/tui/app/session_shell_controller.rs`
- `src/adapter/inbound/tui/app/shell_controller.rs`
- `src/adapter/inbound/tui/app/shell_entrypoint.rs`
- `src/adapter/inbound/tui/app/shell_frontend.rs`
- `src/adapter/inbound/tui/app/shell_layout.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/activity.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/activity_diff/tests.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/directions.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/planning_editor_inputs.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/planning_existing_workspace.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/planning_existing_workspace_inputs.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/planning_init_copy.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/planning_runtime.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/planning_session.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/queue.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/supersession/tests.rs`
- `src/adapter/inbound/tui/app/shell_presentation/prompt_composer.rs`
- `src/adapter/inbound/tui/app/shell_presentation/runtime_status_copy.rs`
- `src/adapter/inbound/tui/app/shell_presentation/session_browser.rs`
- `src/adapter/inbound/tui/app/shell_presentation/session_browser/empty_state.rs`
- `src/adapter/inbound/tui/app/shell_presentation/startup_banner.rs`
- `src/adapter/inbound/tui/app/shell_presentation/status_panels/activity_rail.rs`
- `src/adapter/inbound/tui/app/shell_presentation/status_panels/live_status_layout.rs`
- `src/adapter/inbound/tui/app/shell_presentation/status_panels/parallel_working_copy.rs`
- `src/adapter/inbound/tui/app/shell_presentation/status_panels/plan_indicator.rs`
- `src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_copy.rs`
- `src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_shared.rs`
- `src/adapter/inbound/tui/app/shell_presentation/terminal_text.rs`
- `src/adapter/inbound/tui/app/shell_presentation/transcript_copy.rs`
- `src/adapter/inbound/tui/app/shell_rendering/inline_inspection.rs`
- `src/adapter/inbound/tui/app/shell_rendering/inline_layout.rs`
- `src/adapter/inbound/tui/app/shell_rendering_contract_tests.rs`
- `src/adapter/inbound/tui/app/shell_rendering_contract_tests/planning.rs`
- `src/adapter/inbound/tui/app/shell_rendering_tests.rs`
- `src/adapter/inbound/tui/app/shell_runtime/tests.rs`
- `src/adapter/inbound/tui/app/shell_runtime/tests/flows.rs`
- `src/adapter/inbound/tui/app/shell_runtime/tests/input.rs`
- `src/adapter/inbound/tui/app/shell_runtime/tests/scheduler.rs`
- `src/adapter/inbound/tui/app/test_helpers.rs`
- `src/adapter/inbound/tui/app/tui_testkit.rs`
- `src/adapter/inbound/tui/app/turn_submission_runtime.rs`
- `src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution.rs`
- `src/adapter/inbound/tui/app/view_selection_overlay_ui.rs`
- `src/adapter/inbound/tui/conversation_text.rs`
- `src/adapter/inbound/tui/shell_chrome.rs`
- `src/adapter/inbound/tui/supersession_mud.rs`

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
cargo test shell_rendering::contract_tests
cargo test shell_rendering::tests
cargo test shell_runtime::tests
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```
