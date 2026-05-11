# Terminal UI Testing Methodology

This document turns the reference Codex TUI benchmark into a testing method for this repo. It is
implementation-facing: when the native TUI changes terminal rendering, history insertion, viewport
state, resize behavior, or live-tail presentation, use this as the minimum test design.

## Benchmark Findings

The reference Codex TUI avoids terminal regressions through layered tests rather than by trusting a
single end-to-end terminal run.

The benchmarked layers are:

- terminal primitives tested with fake backends or vt100 buffers
- viewport and history insertion behavior tested independently from app state
- event stream and frame scheduling tested with fake event sources and paused time
- protocol-to-history reducers tested by draining inserted history cells
- user-visible frames tested with snapshots at fixed dimensions

The lesson is that terminal UI bugs are usually state ownership bugs. The tests should prove which
layer owns each state transition: completed transcript, pending history queue, live tail, prompt
cursor, terminal viewport, and resize-triggered frame invalidation.

## Test Pyramid

### 1. Pure Projection Tests

Use this layer for line builders, status copy, overlays, prompt composition, and transcript
projection.

Expected shape:

- no real terminal backend
- no crossterm event loop
- deterministic input structs and rendered `Line` output
- assertions for presence, absence, order, and truncation

Use snapshots only when layout density matters. Prefer direct assertions for small copy changes.

### 2. Reducer And Runtime State Tests

Use this layer for shell input, command dispatch, streaming state, startup/session lifecycle, and
the boundary between committed transcript and live turn state.

Required assertions:

- completed messages move into committed history state
- streaming deltas remain live until turn completion
- clear and thread switch empty pending/deferred history queues
- command-safe buffering does not mutate transcript during unsafe streaming windows
- resize events request a redraw but do not directly mutate conversation state

### 3. Terminal Primitive Tests

Use this layer for any code that writes escape sequences, calls `insert_before`, manipulates
scrollback, clears the screen, or invalidates frame buffers.

Required fixtures:

- a fake or test backend that exposes screen contents
- a vt100-compatible backend when escape sequences matter
- helpers to render buffer contents into plain strings
- helpers to inspect scrollback separately from the active viewport when supported

Required cases:

- insert one committed history block above the viewport
- insert wrapped lines and clear continuation rows
- insert wide characters and verify stale cells are cleared
- insert URL-like lines without breaking clickable text assumptions
- clear visible screen plus scrollback and then redraw a clean header
- reset pending history and prove stale lines cannot flush after reset

### 4. Frame And Viewport Transaction Tests

Use this layer for the frontend draw loop and viewport mode selection.

Required cases:

- `HostScrollback` uses inline viewport and writes new committed history to host scrollback
- `ViewportReplay` does not write committed history to host scrollback
- `ViewportReplay` is explicit-only and keeps inline viewport positioning
- `80x24 -> 80x8 -> 80x24` leaves no duplicate live tail, stale rows, or misplaced prompt
- Windows resize mitigation means suppressing inline viewport append side effects during
  `Terminal::autoresize` and final `Terminal::draw`; see
  [28-reference-codex-tui-rendering-research.md](28-reference-codex-tui-rendering-research.md)
  for the failure mode.
- draw-time `Terminal::draw` autoresize cannot append the live tail into host scrollback
- overlay open/close resets live-tail redraw cache
- hidden tail skips redundant frames but redraws on width and height changes
- frame invalidation forces a full repaint after terminal-side scrolling

### 5. Event And Scheduler Tests

Use this layer when changing crossterm event mapping, redraw requests, background ticks, or live
activity pulsing.

Required cases:

- resize maps to a draw request
- focus gain maps to draw and can refresh palette/theme state
- focus lost does not force a useless frame unless product behavior needs it
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

## One-Contract PR Rule

TUI PRs should stay reviewable by contract owner.

- one PR should have one primary contract: reducer or runtime, terminal primitive, frame scheduling, or visual snapshot
- the PR description should name the touched rows or contract explicitly
- bug-fix PRs should add a reproducer test or capture before the implementation change
- changes that materially modify `shell_rendering`, `ratatui_frontend` or the inline terminal adapter, and `shell_runtime` together are hardening slices and should say so directly in the PR body

## Manual Validation Matrix

Automated tests cannot prove every terminal emulator behavior. Use manual captures for terminal
families when touching escape sequences or viewport policy.

Minimum matrix:

- Linux terminal without multiplexer
- tmux
- Zellij
- Windows Terminal / PowerShell / inline
- Windows Terminal / WSL bash / inline

Optional but valuable:

- Terminal.app
- iTerm2
- VS Code integrated terminal

Manual scenario:

1. start the TUI with an existing conversation or produce at least three committed messages
2. stream a turn long enough to keep a live tail visible
3. resize from a normal height to a short height and back
4. open and close one inspection overlay
5. clear or switch thread
6. confirm the prompt, committed history, and live tail follow the documented mode contract

Capture expectation:

- for meaningful rendering changes, attach a terminal capture or screenshot to the PR when
  practical
- record terminal name, multiplexer, OS, render mode, and viewport mode

When the bug class is prompt echo latency or buffered input delay, record the focused
`prompt-input-delay-pty` profile and summarize it separately:

```bash
bash scripts/capture_native_validation.sh \
  --frontend inline \
  --check-profile prompt-input-delay-pty \
  --terminal "tmux 3.4 detached PTY" \
  --result pass \
  --output-dir docs/validation

bash scripts/summarize_native_validation.sh --check-profile prompt-input-delay-pty
```

## Test File Ownership

Keep tests close to the implementation boundary they protect.

Recommended placement:

- projection tests: beside `shell_presentation` and overlay modules
- shell runtime/event tests: beside `shell_runtime`
- frontend viewport tests: beside `ratatui_frontend`
- terminal adapter tests: beside the terminal adapter once introduced
- broad shell rendering snapshots: `src/adapter/inbound/tui/app/shell_rendering_tests.rs`

Do not grow one monolithic rendering test file for every concern. Split when a test group needs
different fixtures or when failures point to different owners.

## Fixture Design

Add small helpers before adding broad tests.

Recommended fixtures:

- `make_test_app()` for deterministic startup and conversation state
- `append_agent_history_message()` for stable transcript setup
- `set_live_agent_message()` or `push_live_agent_delta()` for live-tail setup
- `render_inline_snapshot(app, width, height)` for single-frame snapshots
- `draw_resize_sequence([(w, h), ...], mode)` for resize regressions
- `screen_text()` and `inline_scrollback_text()` for terminal backend inspection
- `sync_inline_viewport()` for frontend history insertion tests

The fixture should expose the ownership boundary. If a test must reach through many unrelated app
fields, the production boundary is probably too broad.

## Merge Gate

Before merging a terminal rendering change:

- run focused unit and snapshot tests for the touched layer
- run the default focused commands for this repo when the terminal adapter, frame transaction, or reducer boundary changes:
  `cargo test shell_runtime`
  `cargo test inline_terminal_adapter`
  `cargo test history_insertion`
  `cargo test shell_rendering`
- run `cargo fmt`
- run `cargo test` when touching reducers, frontend, or terminal adapter behavior
- run `cargo clippy --all-targets --all-features -D warnings` when touching shared TUI/runtime code
- include manual terminal evidence when the change alters escape sequences, viewport mode, or
  clear or scrollback behavior
- use `scripts/capture_native_validation.sh` or `scripts/capture_native_validation.ps1` for terminal-affecting PRs and attach at least these required rows:
  Windows Terminal / PowerShell / inline
  Windows Terminal / WSL bash / inline

Docs-only research does not require Rust tests, but implementation PRs should not rely on manual
validation alone.

## First Follow-Up Tests For This Repo

The next rendering PR should add these before or with the fix:

1. `ViewportReplay` stays explicit-only, keeps inline viewport positioning, and does not leak rows
   into test backend scrollback during shrink/restore.
2. `HostScrollback` appends only new pending history lines after a completed turn.
3. History window shift inserts only the new suffix once `MAX_CONVERSATION_HISTORY_LINES` is hit.
4. Overlay open after a taller live tail clears stale rows.
5. Clear/thread switch empties pending and remembered history state before the next draw.
6. Draw-time autoresize between viewport sync and frame render does not move the live tail into
   host scrollback.
7. Split `shell_rendering_tests.rs` once snapshot, terminal primitive, or runtime scheduling
   failures need different fixtures or owners.

These are the minimum guardrails for the known disappearing conversation and resize drift bugs.

## Related Docs

- [28-reference-codex-tui-rendering-research.md](28-reference-codex-tui-rendering-research.md)
- [10-inline-scrollback-shell.md](10-inline-scrollback-shell.md)
- [12-platform-validation-matrix.md](12-platform-validation-matrix.md)
