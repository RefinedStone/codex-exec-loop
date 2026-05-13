# Terminal UI Testing Methodology

Use this method when native TUI changes affect terminal rendering, history insertion, viewport
state, resize behavior, overlays, prompt editing, or live-tail presentation.

## Test Layers

### 1. Pure Projection Tests

Use for line builders, status copy, overlays, prompt composition, and transcript projection.

- no real terminal backend
- deterministic input structs and rendered `Line` output
- assertions for presence, absence, order, truncation, and visible key copy
- snapshots only when layout density is the contract

### 2. Reducer And Runtime State Tests

Use for shell input, command dispatch, streaming state, startup/session lifecycle, and the boundary
between committed transcript and live turn state.

Required assertions:

- completed messages move into committed history state
- streaming deltas remain live until turn completion
- clear and thread switch empty pending/deferred history queues
- command-safe buffering does not mutate transcript during unsafe streaming windows
- resize events request redraw without directly mutating conversation state

### 3. Terminal Primitive Tests

Use for code that writes escape sequences, calls `insert_before`, manipulates scrollback, clears the
screen, or invalidates frame buffers.

Required fixtures:

- fake or test backend that exposes screen contents
- vt100-compatible backend when escape sequences matter
- helpers to render buffer contents into plain strings
- helpers to inspect scrollback separately from the active viewport when supported

Required cases:

- insert one committed history block above the viewport
- insert wrapped lines and clear continuation rows
- insert wide characters and verify stale cells are cleared
- clear visible screen plus scrollback and redraw a clean header
- reset pending history and prove stale lines cannot flush after reset

### 4. Frame And Viewport Transaction Tests

Use for frontend draw loop and viewport mode selection.

Required cases:

- `HostScrollback` writes new committed history to host scrollback
- `ViewportReplay` does not write committed history to host scrollback
- `ViewportReplay` stays explicit-only and keeps inline viewport positioning
- shrink/restore frame sequences leave no duplicate live tail, stale rows, or misplaced prompt
- draw-time `Terminal::draw` autoresize cannot append the live tail into host scrollback
- overlay open/close resets live-tail redraw cache
- hidden tail skips redundant frames but redraws on width and height changes
- frame invalidation forces a full repaint after terminal-side scrolling

### 5. Event And Scheduler Tests

Use when changing crossterm event mapping, redraw requests, background ticks, or live activity
pulsing.

Required cases:

- resize maps to a draw request
- focus gain maps to draw and can refresh palette/theme state
- focus lost does not force a frame unless product behavior needs it
- multiple immediate frame requests coalesce into one draw notification
- delayed and immediate frame requests choose the earliest safe draw
- paused/resumed input sources do not steal events from nested terminal programs

### 6. User-Visible Snapshot Tests

Use snapshots for stable surfaces that are hard to validate with a few assertions:

- ready shell
- streaming shell
- viewport replay shell
- queue overlay
- planning editor
- diagnostics/session/help inspection
- narrow-height and narrow-width variants

Snapshot policy:

- keep dimensions explicit in test names or helper calls
- normalize OS-specific paths and terminal capabilities
- avoid snapshots for copy that changes often unless the copy is the contract
- add one targeted assertion near a snapshot for the bug class it protects

## Required Regression Matrix

Every TUI rendering PR should state which rows it touches.

| Area | Required automated proof |
| --- | --- |
| Host scrollback history | pending suffix insert, shifted window insert, no duplicate replay |
| Viewport replay | explicit-only fallback, no host scrollback insert, visible recent transcript, inline viewport contract |
| Resize | shrink/restore frame sequence with no stale rows or duplicated live tail |
| Clear/reset | pending history dropped, viewport reset, fresh header redraw |
| Thread/session switch | old transcript and deferred history cannot leak into new thread |
| Streaming turn | active cell or live delta stays live, final output becomes committed history |
| Overlay | opening overlay clears stale live-tail rows and closing redraws normal tail |
| Terminal fallback | standard and fallback insertion modes each update viewport state correctly |

## Current Automated Entry Points

- `src/adapter/inbound/tui/app/shell_rendering_tests.rs`
- `src/adapter/inbound/tui/app/shell_rendering_contract_tests.rs`
- `src/adapter/inbound/tui/app/inline_terminal_adapter/tests/`
- `src/adapter/inbound/tui/app/shell_runtime/tests/`
- `src/adapter/inbound/tui/app/snapshots/`
- `tests/native_validation_scripts.rs`

## Validation Commands

```bash
. "$HOME/.cargo/env"
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

For TUI visual/presentation work:

```bash
bash scripts/check_tui_layering.sh
```

For broad native/TUI PRs:

```bash
bash scripts/check_native_pr.sh
```

Manual terminal evidence is still required when the change alters escape sequences, viewport mode,
clear behavior, or scrollback behavior. Record manual rows with
`scripts/capture_native_validation.sh` or `scripts/capture_native_validation.ps1`.

## Related Docs

- [README.md](README.md)
- [../plan/10-inline-scrollback-shell.md](../plan/10-inline-scrollback-shell.md)
- [../plan/12-platform-validation-matrix.md](../plan/12-platform-validation-matrix.md)
- [../design/07-tui-layered-architecture-and-aesthetic-contract.md](../design/07-tui-layered-architecture-and-aesthetic-contract.md)
