# Known Gaps And Risk Areas

## Current Constraints
- real terminal behavior still needs manual validation when prompt, streaming, or tail rendering changes
- recent-session loading and blocked startup still gate parts of shell interactivity
- shared runtime access during an active stream still relies on fallback handling for some concurrent requests
- long-session editing and navigation remain intentionally lighter than a mature standalone CLI

## Maintenance Risks
- do not reopen a blank-shell rewrite; the current shell, runtime, and adapter boundaries are the baseline
- keep `src/adapter/inbound/tui/app.rs` near composition and shared state, not renewed feature accumulation
- keep shell rendering changes localized across `ratatui_frontend.rs`, `shell_rendering.rs`, and `shell_presentation.rs` instead of spreading layout policy across many files

## Documentation Rule
Keep this file as a short record of durable constraints. Put new sprint scope in a dedicated feature doc instead of expanding this file into a live backlog.
