# TUI Shell Flow

This file describes the implemented shell shape on `prerelease`.

## Shell Shape
- the app opens directly into a conversation draft on the main screen by default
- inline mode uses host terminal scrollback as the primary history surface
- one tail-anchored live region carries the prompt, transient streaming text, and compact notices
- diagnostics, recent sessions, and template previews render as in-shell inspection surfaces
- alternate-screen remains an explicit fallback frontend, not the default path

## Operator Signals
- the active prompt owns the visible cursor
- streamed assistant text is visible before the turn completes
- routine inline copy hides raw `thread_id` and `turn_id`
- approval, tool, warning, and follow-up notices appear as compact shell activity instead of a heavy persistent footer

## Interaction Model
1. Startup checks begin in the background.
2. The shell is visible immediately with the current workspace path.
3. Once startup diagnostics pass, recent-session loading, prompt submission, and session actions are enabled.
4. During a turn, deltas stay in the live region until completion, then the final assistant text commits into normal history.

## Boundaries
- inline mode remains a Ratatui-driven shell, so prompt, streaming, and tail rendering changes still need real-terminal validation
- shell commands and inspection surfaces can evolve, but the current baseline is scrollback-first history plus one live tail region
