# TUI Shell Flow

This document is intentionally compact. The current shell shape is useful context, but detailed UI behavior is expected to change in phase 2.

## Current Shell Shape
- the app opens directly into a conversation draft on the main screen by default
- inline mode still reserves a `Transcript / tail` region as the primary surface
- the composer stays at the bottom, but the current layout still behaves like one shell frame
- startup diagnostics, recent sessions, and template preview are overlay surfaces opened from shell commands or shortcuts
- input can be buffered before startup finishes, but send and session actions still wait for startup diagnostics to pass
- lightweight transcript navigation exists through `PageUp`, `PageDown`, `Home`, and `End`

## Target Flow
- the terminal should read top-to-bottom like a Spring Boot application log or Codex CLI session
- host terminal scroll and mouse-wheel behavior should be the primary way to inspect earlier output
- inline mode should keep one tail-anchored prompt/live region, not a dedicated transcript viewport in the middle of the screen
- any remaining status chrome should support the flow instead of splitting the screen into stable framed sections

## Current Interaction Model
1. Startup checks begin in the background.
2. The shell is visible immediately with the current workspace path.
3. Once startup diagnostics pass, recent-session loading and prompt submission are enabled.
4. Streamed turn events update transcript, status, and follow-up state inside the shell.

## Design Note
This is the implemented form, not the final UX commitment. Phase 2 can replace or remove overlays, raw-mode assumptions, the `Transcript / tail` viewport, and parts of the current shell chrome as long as the live shell baseline stays intact.
