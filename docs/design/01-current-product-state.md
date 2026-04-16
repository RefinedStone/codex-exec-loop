# Current Product State

`prerelease` ships a shell-first Rust client on `codex app-server`.

## Shipped Surface

- Inline shell is the only frontend.
- Startup diagnostics begin immediately and the shell renders before all checks finish.
- Manual submit can queue while startup is pending, then auto-submit once the shell is ready.
- The client can open a new draft, resume a thread, load snapshots, and stream turns through the shared app-server boundary.
- Shell overlays cover diagnostics, recent sessions, queue state, automation controls, and planning.
- Recent sessions support search, paging, and current-workspace filtering.
- Auto follow-up is planning-queue-driven and no longer loads workspace-defined prompt files.
- Planning is already live: `:planning` stages drafts, opens the embedded editor, promotes accepted files, and exposes queue/proposal state.
- Invalid planning writes are rolled back and can trigger a bounded repair flow.
- Runtime warnings, approval state, tool activity, and optional GitHub review polling are visible in normal shell flow.

## Active Constraints

- Real-terminal validation is still required after prompt, streaming, overlay, or restore changes.
- Some shell actions remain gated by startup diagnostics.
- Non-stream requests can still fall back to isolated runtime access while a main stream is active.
- Planning detail mode supports manual authoring only; the `llm-assisted` branch remains disabled.
- The checked-in schema snapshot still predates newer app-server approval response methods, so the TUI does not expose approve or deny actions yet.
