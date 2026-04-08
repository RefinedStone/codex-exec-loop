# Current Product State

## Phase-1 Baseline
The `prerelease` branch is a working shell-first native client, not a dashboard prototype.

## Shipped Capability
- shell-first startup into a draft conversation on the main terminal screen by default, with `CODEX_EXEC_LOOP_FRONTEND=alternate` as the explicit fullscreen override and legacy `CODEX_EXEC_LOOP_ALT_SCREEN` still accepted as fallback
- startup diagnostics, recent-session browsing, and follow-up template inspection rendered inside the inline shell, with alternate-screen still available as the framed fallback path
- manual prompt submission can queue while startup checks are still running, then auto-send once startup becomes ready
- new-thread start, existing-thread resume, snapshot loading, and streamed turn execution through the app-server flow
- inline shell commands such as `:diag`, `:sessions`, `:templates`, `:new`, and `:help`
- a transitional inline shell shape with a named transcript region above the composer and lightweight viewport navigation
- builtin auto follow-up templates, workspace template loading, inspection-backed stop-keyword editing, and a no-file-change stop rule

## What Still Feels Transitional
- inline mode still dedicates a `Transcript / tail` section above the prompt box, so the shell can still read like a repeated frame instead of ordinary terminal history
- recent-session loading still depends on startup diagnostics passing, and blocked startup still keeps prompt execution from starting
- the shell still runs as a full Ratatui raw-mode viewport and the streaming transcript is not yet a true scrollback-native CLI history
- concurrent non-stream requests can still fall back to an isolated connection while a turn stream is active

## Product Strengths Worth Preserving
- the shell is the primary surface, not a secondary route
- transcript updates and turn progress are already visible in one flow
- auto follow-up is operator-visible and controllable from the UI
- protocol work and filesystem concerns still live behind clear adapter boundaries

## Current Documentation Posture
Treat the first pass as complete enough to serve as the baseline. Phase 2 is expected to change a lot, so docs should preserve durable context and avoid locking in temporary UI details more than necessary.
