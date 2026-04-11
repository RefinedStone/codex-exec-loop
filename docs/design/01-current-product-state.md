# Current Product State

`prerelease` currently ships a shell-first native client built on `codex app-server`.

## Baseline

- inline main-buffer mode is the default frontend; alternate-screen remains explicit opt-in
- startup diagnostics begin immediately and the shell becomes visible before all checks finish
- manual input can buffer while startup is still pending, then auto-submit once the shell reaches a sendable state
- the client can start a new draft, resume an existing thread, load snapshots, and stream new turns through the shared app-server boundary
- inline inspection surfaces cover diagnostics, recent sessions, follow-up templates, and planning
- recent-session browsing supports search, paging, and current-workspace filtering
- auto follow-up ships with builtin templates plus workspace templates from `.codex-exec-loop/followups/`
- the planning feature already exists: `:planning` can stage simple or detail/manual drafts, open the embedded draft editor, promote staged files, and surface queue/proposal status in the shell
- invalid planning task-ledger writes are rolled back and can trigger a bounded repair retry
- approval state, tool activity, runtime warnings, and optional GitHub review polling are visible in routine shell flow

## Current Constraints

- shell rendering still needs real-terminal validation when prompt, streaming, overlay, or restore behavior changes
- recent-session loading and some shell actions remain gated by startup diagnostics
- some non-stream requests still fall back to isolated runtime access while a turn stream is active
- long-session editing and navigation are intentionally lighter than a mature standalone CLI
- planning detail mode only supports manual draft authoring today; the `llm-assisted` branch is visible but disabled
