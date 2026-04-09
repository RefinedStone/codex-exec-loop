# Current Product State

The `prerelease` branch is a shell-first native client built around `codex app-server`.

## Current Baseline
- shell-first startup into a draft conversation on the main terminal screen by default, with `CODEX_EXEC_LOOP_FRONTEND=alternate` as the explicit fullscreen override and legacy `CODEX_EXEC_LOOP_ALT_SCREEN` still accepted as fallback
- startup diagnostics, recent-session browsing, and follow-up template inspection rendered inside the inline shell, with alternate-screen still available as the framed fallback path
- manual prompt submission can queue while startup checks are still running, then auto-send once startup becomes ready
- new-thread start, existing-thread resume, snapshot loading, and streamed turn execution through the app-server flow
- inline shell commands such as `:diag`, `:sessions`, `:templates`, `:new`, and `:help`
- host terminal scrollback used as the primary history surface in inline mode, with one tail live region for prompt, transient streaming text, and compact notices
- visible cursor ownership in the active inline prompt and visible streamed agent text before completion
- compact routine status copy that hides raw protocol ids from normal inline flow
- builtin auto follow-up templates, workspace template loading, inspection-backed stop-keyword editing, and a no-file-change stop rule
- session query, paging, recent-project filtering, and GitHub review-change notices inside the shell

## Current Constraints
- terminal behavior still depends on host terminal capabilities and should be manually validated when shell rendering changes
- recent-session loading, prompt execution, and some shell actions still depend on startup diagnostics passing
- some non-stream requests can still fall back to isolated runtime access while a turn stream is active
- long-session editing and navigation remain intentionally simpler than a mature dedicated CLI

## Documentation Posture
Describe the implemented baseline, not a future UX script. When a new sprint changes shell shape, runtime behavior, or automation behavior in a material way, open a dedicated feature doc instead of turning this file into a rolling history log.
