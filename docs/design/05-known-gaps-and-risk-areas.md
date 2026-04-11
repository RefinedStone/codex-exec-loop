# Known Gaps And Risk Areas

## Durable Constraints

- prompt, streaming, overlay, and restore behavior still need manual terminal validation
- startup diagnostics still gate recent-session loading and parts of shell interactivity
- shared runtime access during an active stream still depends on fallback handling for some concurrent requests
- long-session editing and transcript navigation remain intentionally lighter than a mature standalone CLI
- planning detail mode does not support `llm-assisted` authoring yet

## Maintenance Risks

- do not reopen a blank-shell rewrite; the current shell and runtime are the baseline
- keep `src/adapter/inbound/tui/app.rs` near composition and shared state
- keep shell layout policy localized to `ratatui_frontend.rs`, `shell_rendering.rs`, and `shell_presentation.rs`
- keep planning validation and reconciliation in application services instead of leaking file-policy logic into the TUI
