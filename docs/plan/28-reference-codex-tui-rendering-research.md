# Reference Codex TUI Rendering Research

This note summarizes what is worth borrowing from the reference Codex TUI under
`/home/akra/codex-exec-loop/reference/codex/codex/codex-rs/tui`, with focus on
conversation history durability, inline viewport rendering, and terminal resize behavior.

## Problem Fit

The current native TUI has two related failure classes:

- OS or terminal environment changes can switch rendering behavior, after which completed
  conversation lines appear to disappear.
- Resizing the terminal smaller and then restoring it can leave the inline shell laid out against
  stale viewport assumptions.

Both point to the same architectural risk: the host terminal, Ratatui inline viewport, and app
conversation model do not have a single explicit owner for what is stable transcript history and
what is transient live UI.

## Reference Model

The reference Codex TUI does not rely on plain `ratatui::Terminal` for its inline shell. It carries a
derived custom terminal in `src/custom_terminal.rs` and wraps it through `src/tui.rs`.

Important traits of that model:

- The terminal wrapper owns `viewport_area`, `last_known_screen_size`, `last_known_cursor_pos`, and
  `visible_history_rows`.
- A draw pass calls `pending_viewport_area()`, adjusts viewport position when a resize changes the
  cursor row, updates the inline viewport height, flushes pending transcript lines, invalidates the
  diff buffer when raw terminal scrolling was used, and only then renders the live frame.
- The draw pass runs inside `stdout().sync_update(...)`, so viewport movement, history insertion,
  clear operations, and frame flush are one terminal transaction.
- Completed transcript cells are converted into display lines and queued through
  `Tui::insert_history_lines`. They are flushed above the viewport by `insert_history_lines_with_mode`
  instead of being redrawn as part of the live frame.
- The live frame owns the active cell, prompt, footer/status, overlays, and transient stream output.
  When an active cell finishes, it becomes a history cell and is inserted above the viewport.
- `InsertHistoryMode::Standard` uses scroll regions plus reverse index to make room above the
  viewport. `InsertHistoryMode::Zellij` falls back to newlines at the screen bottom because Zellij
  does not support the same scroll-region behavior.
- Full UI clears drop queued history first, hard-reset visible screen plus scrollback with one ANSI
  sequence, reset the inline viewport to the top, and clear transcript bookkeeping.
- Resize and focus events are mapped to draw requests by the event stream. Draw requests are
  coalesced through `FrameRequester` and clamped by a small frame-rate limiter.
- Alternate screen is treated as optional. The default can be auto-disabled in Zellij because
  Zellij does not preserve scrollback in alternate-screen buffers.

The useful principle is not just "use custom terminal code". It is that the terminal viewport is a
first-class state machine, and transcript insertion is part of the terminal transaction, not a side
effect hidden behind a generic Ratatui draw call.

## Current Native TUI Model

The current native frontend in `src/adapter/inbound/tui/app/ratatui_frontend.rs` uses
`ratatui::Terminal` with render-mode-specific viewport options.

Current behavior:

- `InlineHistoryRenderMode::HostScrollback` is the default. It inserts completed history lines above
  a `Viewport::Inline(INLINE_VIEWPORT_HEIGHT)` viewport via `terminal.insert_before(...)`.
- `InlineHistoryRenderMode::ViewportReplay` is selected automatically for Windows or `WT_SESSION`,
  or explicitly through `CODEX_EXEC_LOOP_INLINE_HISTORY_MODE`. In this mode the app remembers
  history, skips host scrollback insertion, keeps `Viewport::Inline(...)` so the shell remains
  anchored below the invoking prompt, and suppresses Ratatui's resize-time `append_lines` call so
  stale frame rows are not pushed into scrollback during resize.
- `sync_inline_viewport()` calls `terminal.autoresize()`, computes the current logical history
  lines, inserts only the pending suffix in host-scrollback mode, and asks whether the live tail
  signature changed.
- `ShellRuntime` maps `Event::Resize` to a redraw request, but it does not own cursor-position
  reconciliation or viewport invalidation beyond Ratatui's `autoresize()`.
- The live tail cache keys on terminal width, height, and rendered tail lines. That prevents many
  redundant frames, but it does not repair host scrollback state after a terminal moves or clears
  inline content.

This already follows part of the reference direction: completed lines are separated from live tail
rendering. The weaker point is that the terminal behavior is delegated to Ratatui's inline viewport,
while the app still tries to support multiple history semantics depending on OS and environment.

## Risk Analysis

### Conversation Disappearing

In host-scrollback mode, completed history is no longer drawn by the live frame after insertion. If a
terminal or multiplexer drops, rewrites, or fails to preserve the inserted scrollback, the app can
only redraw the tail. The logical transcript still exists in `ConversationViewModel`, but the
frontend's `InlineHistoryState` believes it already inserted those lines and will not replay them
unless it detects a full reset.

In viewport-replay mode, host scrollback insertion is intentionally skipped. A fullscreen viewport
was tested as a resize workaround, but it breaks the inline shell contract by repainting from the
top of the terminal instead of staying below the invoking prompt. Keeping the inline viewport while
suppressing resize-time `append_lines` avoids that regression, but it still does not create durable
scrollback history. The tail mirrors only a bounded recent transcript, so older lines disappearing
from the visible terminal is expected unless the app adds its own scrollable transcript viewport.
The automatic Windows and Windows Terminal switch therefore changes the product contract: one
environment preserves history in terminal scrollback, another shows only a replay window plus live
tail.

### Resize Drift

The reference code treats resize as more than a redraw. It checks the real terminal size, compares
the last known cursor position with the queried cursor position, offsets the inline viewport when the
terminal moved the cursor, and resets the diff buffer when raw scrolling makes the previous buffer
untrustworthy.

The current code requests a redraw and calls `autoresize()`. That can resize Ratatui buffers, but it
does not explicitly decide whether the inline viewport should move, whether old rows need to be
cleared, or whether previously inserted history should be considered invalid. This explains why a
shrink and restore can leave stale rows or wrong anchors.

### Width And Wrapping

Host scrollback lines are inserted using the width at insertion time. After a later resize, the
terminal emulator owns any physical wrapping for those old rows. The app stores logical lines in
`InlineHistoryState`, so it can calculate new pending suffixes, but it cannot reflow already
inserted host scrollback. That is acceptable only if host scrollback is treated as immutable history
and the live viewport is always repaired independently.

## Borrowable Design Decisions

Adopt these ideas before adding more presentation features:

1. Make the terminal viewport an explicit adapter-owned state object.
   Track screen size, viewport rect, last cursor position, visible inserted history rows, and whether
   the previous frame buffer is trustworthy.

2. Flush committed transcript lines during the draw transaction.
   Keep a pending history queue and write it above the viewport immediately before rendering the live
   frame. A history insert and a frame draw should not be separate terminal operations.

3. Separate stable transcript and live UI as a hard contract.
   Completed messages, finalized tool calls, warnings, and session headers become committed history.
   Streaming deltas, active tool cells, prompt input, overlays, and compact status stay in the live
   viewport.

4. Treat viewport replay as a different product mode, not an invisible OS workaround.
   If Windows Terminal needs replay mode, it needs an explicit scrollable transcript surface or a
   clear limitation that only recent transcript lines are visible. Otherwise users will keep seeing
   "lost" conversation history that is actually mode behavior.

5. Handle resize with cursor-aware viewport repair.
   On size change, query cursor position if possible, adjust viewport origin when the host moved the
   cursor, clear the affected viewport area, and invalidate the back buffer before rendering.

6. Add terminal-specific history insertion modes.
   Keep the default scroll-region path, add a Zellij-style fallback when scroll regions are not
   reliable, and avoid hard-coding broad OS decisions until a terminal capability check or validation
   matrix proves the fallback is needed.

7. Implement a real clear/thread-switch reset.
   Clear queued history, reset rendered-history tracking, clear visible screen plus scrollback in one
   sequence, set viewport `y` back to zero, then redraw the fresh session header.

## Recommended Rollout

1. Codify the contract before changing rendering code.
   Update `docs/plan/10-inline-scrollback-shell.md` to say whether host scrollback is the canonical
   history surface on each supported terminal class and what viewport replay guarantees.

2. Add regression tests around the existing frontend.
   Cover `HostScrollback` and `ViewportReplay` separately. The minimum cases are: append history,
   draw live tail, resize smaller, draw, resize back, draw, and assert that stale tail rows are gone
   and the expected transcript contract still holds.

3. Introduce a small terminal adapter in the TUI inbound boundary.
   Start with viewport bookkeeping, clear/reset, and frame invalidation. Do not move application or
   domain types into it.

4. Move history insertion behind that adapter.
   Replace direct `Terminal::insert_before` calls with an explicit history insertion path that can
   select standard, Zellij, or future Windows fallback behavior.

5. Revisit automatic `ViewportReplay`.
   Do not use fullscreen as an invisible Windows workaround; it breaks inline shell positioning.
   Replay mode still needs an owned scrollable transcript viewport or an explicit diagnostic
   contract before it remains the automatic Windows behavior.

6. Validate manually against the target matrix.
   Use Linux terminal, tmux, Zellij, Windows Terminal, WSL inside Windows Terminal, and macOS
   Terminal.app or iTerm2 if available. Capture shrink/restore and long-turn transcript behavior.

## Acceptance Criteria For A Follow-Up Rendering PR

- Resize smaller and restore does not leave stale rows, duplicate tails, or a misplaced prompt.
- Completed conversation history remains available according to the documented mode contract.
- Host-scrollback mode never redraws committed transcript inside the live tail except for startup or
  explicit diagnostic replay.
- Viewport-replay mode either has a scrollable transcript viewport or is not selected
  automatically.
- Clear and thread switch drop pending history and cannot flush stale lines after the reset.
- Tests cover resize redraw, pending history suffix detection, shifted history windows, and the
  selected terminal fallback mode.

## Source References

- Reference terminal wrapper: `reference/codex/codex/codex-rs/tui/src/custom_terminal.rs`
- Reference TUI runtime: `reference/codex/codex/codex-rs/tui/src/tui.rs`
- Reference history insertion: `reference/codex/codex/codex-rs/tui/src/insert_history.rs`
- Reference history dispatch: `reference/codex/codex/codex-rs/tui/src/app/event_dispatch.rs`
- Reference clear/reset helpers: `reference/codex/codex/codex-rs/tui/src/app/history_ui.rs`
- Current frontend: `src/adapter/inbound/tui/app/ratatui_frontend.rs`
- Current render mode selection: `src/adapter/inbound/tui/app.rs`
- Current shell runtime resize handling: `src/adapter/inbound/tui/app/shell_runtime.rs`
- Current rendering tests: `src/adapter/inbound/tui/app/shell_rendering_tests.rs`
