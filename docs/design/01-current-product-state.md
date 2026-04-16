# Current Product State

`prerelease` currently ships a shell-first personal execution cockpit built on `codex app-server`.

## Product Identity

- The product is optimized for long-lived solo work sessions, not one-shot prompt submission.
- Inline shell is the primary surface and the host terminal scrollback is the durable history view.
- The distinctive value is continuity across startup diagnostics, session resume, planning files, and queue-driven auto follow-up inside one terminal.
- Planning is not an optional side panel anymore. Accepted planning state shapes follow-up preview, queue inspection, and post-turn automation.

## Shipped Surface

- Inline shell is the only frontend.
- Startup diagnostics begin immediately and the shell can render before all checks finish.
- Manual submit can queue while startup is pending, then auto-submit once the shell is ready.
- The client can start a new draft, resume a thread, load snapshots, and stream turns through the shared app-server boundary.
- Shell overlays cover diagnostics, recent sessions, queue state, automation controls, planning workspace controls, and directions maintenance.
- Recent sessions support search, paging, and current-workspace filtering.
- Auto follow-up is planning-queue-driven and no longer loads workspace-defined prompt files.
- Planning is live: `:planning` stages drafts, opens the embedded editor, promotes accepted files, and exposes queue and proposal state.
- Shared planning lifecycle commands are live: `akra doctor`, `akra init`, `akra reset`, `:doctor`, `:init`, and `:reset` all use the same validation, bootstrap, and reset rules.
- Invalid planning writes are rolled back and may trigger a bounded repair flow.
- Runtime warnings, approval-review state, tool activity, and optional GitHub review polling are visible in normal shell flow.

## Current Operating Model

- Startup model:
  Startup diagnostics establish whether the shell can submit, browse sessions, and start planning-driven work.
- Session model:
  The operator either starts from a blank draft or resumes an existing thread through the session browser.
- Planning model:
  The operator owns `.codex-exec-loop/planning/` files through staged drafts and explicit promotion.
- Automation model:
  Post-turn automation only acts on accepted planning state, queue head availability, and explicit stop rules.
- Recovery model:
  Invalid planning changes are restored, archived, and surfaced back to the operator instead of being silently accepted.

## Active Constraints

- Real-terminal validation is still required after prompt, streaming, overlay, or restore changes.
- Some shell actions remain gated by startup diagnostics.
- Non-stream requests can still fall back to isolated runtime access while a main stream is active.
- Planning detail mode supports manual authoring only; the `llm-assisted` branch remains disabled.
- The checked-in schema snapshot still predates newer app-server approval response methods, so the TUI does not expose approve or deny actions yet.

## Current Product Risks

- The operator mental model spans startup state, shell state, planning state, queue state, proposal state, repair state, and auto-follow state at once.
- Planning is powerful but still expensive to author for a casual workspace because draft, promote, queue-idle, and directions concepts arrive early.
- Auto follow-up pause reasons are technically accurate but still read like system state rather than operator guidance.
- Overlay coverage is broad, but the current surface does not always make it obvious what the next recoverable action is.
- Approval review is visible but still requires out-of-band action.

## Code Entry

- Shell runtime entrypoint: `src/adapter/inbound/tui/app`
- Planning feature entrypoint: `src/adapter/inbound/tui/app/planning`
- Application planning services: `src/application/service/planning`
