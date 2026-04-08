# Current Product State

## What The Latest Branch Already Does
The `prerelease` branch is no longer just a dashboard prototype. It now supports:

- shell-first startup into a new conversation draft
- startup diagnostics available as a shell overlay
- recent session browsing available as a shell overlay
- startup checks and account diagnostics
- recent session browsing from `thread/list`
- thread history loading from `thread/read`
- new thread start and existing thread resume
- prompt submission through `turn/start`
- streamed agent deltas and completed items rendered in the shell
- normal conversation flow stays on the main terminal screen by default, with `CODEX_EXEC_LOOP_ALT_SCREEN=1` as an opt-in fallback
- the composer is the bottom-most shell surface, with key hints folded into existing panel titles instead of a dedicated controls pane
- the composer now accepts inline shell commands such as `:diag`, `:sessions`, `:templates`, `:new`, and `:help` while detailed inspection still lives in overlays
- lightweight transcript viewport navigation with `PageUp`, `PageDown`, `Home`, and `End`
- builtin auto follow-up strategies
- workspace follow-up templates loaded from `.codex-exec-loop/followups/`
- auto-stop rules for `AUTO_STOP` and no-file-change turns

## Why The UX Still Feels Different From Codex CLI
Even with live shell behavior, the app still feels more page-based than Codex CLI because:

- startup diagnostics and recent-session browsing still open as modal overlays
- prompt sending is still gated on startup diagnostics instead of sharing one continuous runtime state
- concurrent request actions still fall back to an isolated connection while a turn stream is active
- there is still no fully continuous runtime that keeps every shell action attached to exactly one transport process

## Current Strengths
- the shell is now the default landing surface instead of a later navigation target
- startup status and recent sessions are reachable without leaving the shell
- the shell already renders real transcript updates
- auto follow-up is visible and controllable from the UI
- the codebase still follows a clear hexagonal split
- the app-server protocol work is kept behind one outbound adapter

## Current Refactor Stage
The product-facing UX pivot and the planned P3 code-health cleanup are largely complete. Current follow-up work should stay in opportunistic maintenance, not another shell UX redesign.

- the main shell behavior is already on `prerelease`
- the current effort is reducing the size and mixed responsibilities of `app.rs`
- recent refactors already pulled rendering, presentation, layout, viewport, shell controller, conversation-model state, and the TUI runtime/background event loop into dedicated modules
- `app.rs` is now primarily the composition root for shared state plus shell-specific builder helpers and tests
- the remaining work is now mostly opportunistic maintenance rather than another standing refactor milestone

## Immediate Documentation Goal
All docs should assume the current branch already has streaming shell behavior and auto follow-up. Future planning should build on that baseline instead of describing the older placeholder shell.
