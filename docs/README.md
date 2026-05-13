# Docs Map

Use this map to find the current implementation truth quickly. The docs are intentionally compact:
implemented behavior belongs in current contract and design notes; long roadmaps, future backlog,
and one-off research notes are not kept in this tree.

## Read First

- [../README.md](../README.md): product surface, install, commands, development, and diagnostics
- [supersession/current-contract.md](supersession/current-contract.md): shipped planning,
  continuation, and parallel-mode operator contract
- [agent/README.md](agent/README.md): compact Codex agent reference map

## Current Design

- [design/01-current-product-state.md](design/01-current-product-state.md): product identity,
  surface map, runtime shape, and code entry
- [design/02-tui-shell-flow.md](design/02-tui-shell-flow.md): operator-visible shell modes,
  conversation flow, planning/continuation flow, and recovery states
- [design/04-hexagonal-runtime-architecture.md](design/04-hexagonal-runtime-architecture.md):
  `core`, `application`, `domain`, and adapter dependency rules and current boundary ownership
- [design/05-parallel-control-plane-architecture.md](design/05-parallel-control-plane-architecture.md):
  parallel-mode control-plane ownership, R6 runtime decision, and projection rules
- [design/06-planning-runtime-and-draft-editor.md](design/06-planning-runtime-and-draft-editor.md):
  DB-backed planning authority, staged draft promotion, runtime task intake, and recovery rules
- [design/07-tui-layered-architecture-and-aesthetic-contract.md](design/07-tui-layered-architecture-and-aesthetic-contract.md):
  TUI layer ownership, theme rules, and visual editing guardrails
- [design/08-parallel-mode-supersession-board.md](design/08-parallel-mode-supersession-board.md):
  shipped parallel-mode board and selected-detail timeline shape

## Operations

- [plan/04-worktree-branch-rules.md](plan/04-worktree-branch-rules.md): branch naming, worktree,
  review, merge, and cleanup rules
- [plan/10-inline-scrollback-shell.md](plan/10-inline-scrollback-shell.md): inline shell and host
  scrollback contract
- [plan/11-parallel-worktree-plan.md](plan/11-parallel-worktree-plan.md): live parallel worktree
  coordination snapshot
- [plan/12-platform-validation-matrix.md](plan/12-platform-validation-matrix.md): platform terminal
  validation matrix and capture profiles
- [plan/13-native-packaging-and-operator-runbook.md](plan/13-native-packaging-and-operator-runbook.md):
  native bundle, npm, release, and operator handoff runbook
- [validation/README.md](validation/README.md): real validation artifact index
- [validation/terminal-ui-testing-methodology.md](validation/terminal-ui-testing-methodology.md):
  terminal UI test design for rendering, scrollback, resize, and snapshots

## Implemented Surfaces

- TUI: Ratatui/Crossterm inline shell in `src/adapter/inbound/tui/`, including sessions,
  diagnostics, planning, directions, queue, model/think controls, follow-up, and parallel-mode
  overlays.
- Core runtime: headless app command/effect/completion/snapshot coordination in `src/core/`,
  separated from TUI-only shell code so inbound adapters can share the same lifecycle boundary.
- CLI: `akra doctor`, `akra status`, `akra queue`, `akra reset`, `akra planning-tool`,
  `akra parallel-tick`, `akra admin`, and `akra telegram` dispatch through
  `src/adapter/inbound/cli.rs`.
- Admin: Axum/Askama planning admin UI and JSON API in `src/adapter/inbound/admin_api/`, with
  templates under `templates/admin/` and packaged assets under `assets/admin/`.
- Telegram: control-plane inbound adapter in `src/adapter/inbound/telegram_bot/` and HTTP outbound
  adapter in `src/adapter/outbound/telegram/`.
- Planning runtime: services under `src/application/service/planning/`, persisted by SQLite
  authority adapters and mirrored to planning workspace files.
- Parallel mode: services under `src/application/service/parallel_mode/` with git worktree, GitHub
  delivery, lease/session detail, distributor, and runtime event storage boundaries.
- Packaging: native release scripts, GitHub Actions, and npm wrapper packages under `scripts/`,
  `.github/workflows/`, and `npm/`.

## Rules

- Keep current operator behavior in `docs/supersession/current-contract.md`.
- Keep technical depth in `docs/design/` and operational workflow in `docs/plan/`.
- Do not reintroduce future implementation backlogs into `docs/plan/`; open issues or task
  planning should live outside the repo docs unless it is part of a current PR contract.
- Prefer links to current truth over repeating the same contract in multiple places.
