# Current Product State

## Phase-1 Baseline
The `prerelease` branch is a working shell-first native client, not a dashboard prototype.

## Shipped Capability
- shell-first startup into a draft conversation on the main terminal screen by default, with `CODEX_EXEC_LOOP_ALT_SCREEN` as an opt-in fallback
- startup diagnostics and recent-session browsing exposed as shell overlays, with recent-session loading still gated by startup diagnostics
- new-thread start, existing-thread resume, snapshot loading, and streamed turn execution through the app-server flow
- inline shell commands such as `:diag`, `:sessions`, `:templates`, `:new`, and `:help`
- single-column transcript with a bottom composer and lightweight viewport navigation
- builtin auto follow-up templates, workspace template loading, overlay-backed stop-keyword editing, and a no-file-change stop rule

## What Still Feels Transitional
- diagnostics, sessions, and template browsing are still modal overlays
- prompt submission and recent-session loading still depend on startup diagnostics passing, even though the composer can buffer input earlier
- the shell still runs as a full Ratatui raw-mode viewport rather than a true scrollback-native CLI
- concurrent non-stream requests can still fall back to an isolated connection while a turn stream is active

## Product Strengths Worth Preserving
- the shell is the primary surface, not a secondary route
- transcript updates and turn progress are already visible in one flow
- auto follow-up is operator-visible and controllable from the UI
- protocol work and filesystem concerns still live behind clear adapter boundaries

## Current Documentation Posture
Treat the first pass as complete enough to serve as the baseline. Phase 2 is expected to change a lot, so docs should preserve durable context and avoid locking in temporary UI details more than necessary.
