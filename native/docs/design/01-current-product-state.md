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

## Immediate Documentation Goal
All docs should assume the current branch already has streaming shell behavior and auto follow-up. Future planning should build on that baseline instead of describing the older placeholder shell.
