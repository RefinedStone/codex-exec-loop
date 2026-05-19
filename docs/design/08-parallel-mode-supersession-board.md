# Parallel Mode Supersession Board

This file describes the shipped parallel-mode board shape. It is not a future UI plan.

The board is a TUI projection over the application control-plane. It must not calculate dispatch,
capacity, retry, worker launch, or supersession policy itself. See
[`05-parallel-control-plane-architecture.md`](./05-parallel-control-plane-architecture.md) for the
ownership contract.

## Current Shape

The board is a dense operations view inside the native TUI. It combines:

- readiness state
- fixed local worktree pool state
- active agent roster
- selected agent/session detail
- compact lifecycle timeline
- distributor head and delivery boundary
- accepted queue state and dispatch-withheld reason

The metaphor is intentionally operational: slot/lane board for current parallelism, selected
timeline for session transitions, and distributor corridor for serialized delivery.

## Implementation Entry

- TUI command entry: `src/adapter/inbound/tui/app/parallel_mode.rs`
- Popup projection/copy: `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/supersession.rs`
- Application control-plane: `src/application/service/parallel_mode/control_plane/`
- Parallel-mode service boundary: `src/application/service/parallel_mode/`
- Domain projection rules: `src/domain/parallel_mode/`
- Rendering contract tests: `src/adapter/inbound/tui/app/shell_rendering_contract_tests/`

## View Contract

- `:parallel` or `:pa` opens the board and attempts enable/refresh through the control-plane
  service.
- `:parallel off` or `:pa off` disables local parallel mode.
- `Esc`, `Ctrl+c`, or `Ctrl+o` close the board surface without disabling parallel mode.
- `Ctrl+r` refreshes readiness.
- `Ctrl+p` is the local off-switch while the board owns input.
- `Tab`, arrows, `Enter`, and `Space` navigate and inspect selectable board rows.

Displayed shortcut copy must match these implemented input paths.
The first visible command-hint row must include the global board controls (`Ctrl+r`,
`Ctrl+p` when enabled, `:peek`, and close shortcuts), because compact inline rendering can clip the
hint panel to a single body row.

## Projection Rules

- Keep topology and chronology separate: current pool/roster state first, selected lifecycle
  timeline second.
- Reuse persisted session lifecycle terms such as `assigned`, `starting`, `running`,
  `reported_complete`, `ledger_refreshing`, `commit_ready`, and `merge_queued`.
- Keep delivery history read-only. Source push, PR automation, integration, and cleanup are
  distributor boundaries, not direct TUI mutation controls.
- Bound branch, worktree, queue note, and runtime event fields before they reach dense panels so
  narrow terminal snapshots remain stable.
