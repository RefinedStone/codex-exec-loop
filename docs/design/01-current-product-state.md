# Current Product State

This file is a supporting product snapshot for identity, surface map, and code entry.

The canonical current contract for supersession, planning, and directions lives in
[../supersession/current-contract.md](../supersession/current-contract.md).

## Product Identity

- The product is optimized for long-lived solo work sessions, not one-shot prompt submission.
- Inline shell is the primary surface and the host terminal scrollback is the durable history view.
- The distinctive value is continuity across startup diagnostics, session resume, planning files, and queue-driven continuation inside one terminal.
- Planning is not an optional side panel anymore. Accepted planning state shapes follow-up preview, queue inspection, and internal post-turn continuation.

## Surface Map

- Inline shell is the only frontend.
- Startup diagnostics, session resume, queue inspection, planning, directions, and
  supersession supervision all stay inside that one shell.
- Planning is part of the main operator loop rather than an optional side workflow.
- Accepted planning shapes queue state, follow-up preview, and continuation behavior.
- Supersession adds the worker pool, supervisor board, and distributor delivery lane on top of the
  same shell model.

## Runtime Shape

- Startup establishes whether the shell can submit, browse sessions, and enter planning-driven work.
- The operator moves between blank drafts, resumed threads, inspection overlays, and authoring
  flows without leaving the same terminal session.
- Accepted planning remains operator-owned through staged drafts and explicit promotion.
- Invalid planning writes are restored, archived, and surfaced back to the operator instead of
  being silently accepted.
- Git-backed planning authority is repo-scoped and store-backed under the user-level
  `.akra/projects/<repo-hash>/runtime/` directory.

## Active Constraints

- Real-terminal validation is still required after prompt, streaming, overlay, or restore changes.
- Some shell actions remain gated by startup diagnostics.
- Non-stream requests can still fall back to isolated runtime access while a main stream is active.
- Planning detail mode supports manual authoring only; the `llm-assisted` branch remains disabled.
- The checked-in schema snapshot still predates newer app-server approval response methods, so the TUI does not expose approve or deny actions yet.

## Code Entry

- Shell runtime entrypoint: `src/adapter/inbound/tui/app.rs`
- Supersession shell entrypoint: `src/adapter/inbound/tui/app/parallel_mode.rs`
- Supersession application services: `src/application/service/parallel_mode/`
- Session browser domain rules: `src/domain/session_browser.rs`
- Session catalog application service: `src/application/service/session_service.rs`
- Planning authority port: `src/application/port/outbound/planning_authority_port.rs`
- Planning authority adapter: `src/adapter/outbound/db/sqlite_planning_authority_adapter.rs`
- Planning feature entrypoint: `src/adapter/inbound/tui/app/planning/`
- Application planning services: `src/application/service/planning/`
