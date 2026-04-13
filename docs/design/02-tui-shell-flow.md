# TUI Shell Flow

This file describes the implemented shell shape on `prerelease`.

## Shell Shape

- the app opens into a draft conversation immediately
- inline mode uses host terminal scrollback as the primary history surface
- one live tail region carries the active prompt, transient streaming text, and compact notices
- blank startup reads as startup context, conversation placeholder, and prompt
- diagnostics, sessions, templates, planning, and queue render as in-shell inspection surfaces
- inline is the only shipped frontend path

## Commands And Keys

- inline shell commands: `:diag`, `:sessions`, `:templates`, `:queue`, `:stop`, `:planning`, `:new`, `:help`
- prompt editing: `Enter` send, `Ctrl+j` newline, `Ctrl+u` clear, `Ctrl+w` delete previous word
- shell navigation: `Ctrl+t` new draft, `Ctrl+C` back, `Ctrl+q` quit
- follow-up inspection and control stay in-shell instead of opening a separate modal app

## Operator Signals

- the active prompt owns the visible cursor
- streamed assistant text is visible before the turn completes
- committed transcript history moves into normal scrollback once a turn finishes
- routine status copy hides raw `thread_id` and `turn_id`
- approval, tool, warning, and planning notices stay compact instead of living in a heavy footer

## Interaction Model

1. Startup checks begin in the background.
2. The shell becomes editable immediately.
3. Startup-ready state unlocks recent sessions, normal prompt submission, and session switching.
4. During a turn, the live tail shows streaming deltas and preserves buffered input.
5. Planning and other inspections reuse the same shell surface instead of switching products.

## Boundaries

- inline mode is still Ratatui-driven, so prompt, streaming, and restore changes need real-terminal validation
- the current contract is scrollback-first history plus one live tail region; future UI changes should preserve that baseline unless the product direction changes explicitly
