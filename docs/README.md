# Docs Map

Use this map to find current truth quickly. Most changes should start with one current doc plus, at
most, one supporting deep dive.

## Read First

- [supersession/README.md](supersession/README.md): current supersession, planning, and directions hub
- [supersession/current-contract.md](supersession/current-contract.md): shipped operator-facing contract
- [supersession/remaining-work.md](supersession/remaining-work.md): unfinished or lightly validated work
- [agent/README.md](agent/README.md): compact Codex agent reference map
- [plan/20-context-first-architecture-and-doc-coherence.md](plan/20-context-first-architecture-and-doc-coherence.md): current architecture/docs roadmap

## Architecture

- [design/04-hexagonal-runtime-architecture.md](design/04-hexagonal-runtime-architecture.md): dependency direction, boundary rules, and small-context constraints
- [plan/17-structure-and-architecture-debt-map.md](plan/17-structure-and-architecture-debt-map.md): current hotspot order and completed boundary checkpoints
- [design/06-planning-runtime-and-draft-editor.md](design/06-planning-runtime-and-draft-editor.md): planning runtime and draft editor details
- [design/07-tui-layered-architecture-and-aesthetic-contract.md](design/07-tui-layered-architecture-and-aesthetic-contract.md): TUI layer ownership and visual contract
- [design/08-parallel-mode-mud-timeline-ui-pack-research.md](design/08-parallel-mode-mud-timeline-ui-pack-research.md): Parallel Mode MUD timeline UI research brief

## Operations

- [releases/v1.2.9-to-prerelease.md](releases/v1.2.9-to-prerelease.md): release delta from `v1.2.9`
- [validation/README.md](validation/README.md): validation artifact index
- [plan/12-platform-validation-matrix.md](plan/12-platform-validation-matrix.md): platform checks
- [plan/13-native-packaging-and-operator-runbook.md](plan/13-native-packaging-and-operator-runbook.md): packaging and operator runbook
- [plan/04-worktree-branch-rules.md](plan/04-worktree-branch-rules.md): GitHub/worktree workflow rules
- `scripts/check_native_pr.sh`: local native PR gate for layering, rustfmt check, tests, and clippy
- `scripts/planning-tool.sh`: wrapper for the structured planning task tool surface

## Implemented Surfaces

- TUI: Ratatui/Crossterm inline shell in `src/adapter/inbound/tui/`, including sessions, diagnostics, planning, directions, queue, task intake, follow-up, and parallel-mode overlays.
- CLI: `akra doctor`, `akra reset`, `akra planning-tool`, `akra parallel-tick`, `akra admin`, and `akra telegram` dispatch through `src/adapter/inbound/cli.rs`.
- Admin: Axum/Askama planning admin UI and JSON API in `src/adapter/inbound/admin_api/`, with templates under `templates/admin/` and packaged assets under `assets/admin/`.
- Planning runtime: services under `src/application/service/planning/`, persisted by SQLite authority adapters and mirrored to planning workspace files.
- Parallel mode: services under `src/application/service/parallel_mode/` with git worktree, GitHub delivery, lease/session detail, distributor, and runtime event storage boundaries.
- Packaging: native release scripts, GitHub Actions, and npm wrapper packages under `scripts/`, `.github/workflows/`, and `npm/`.

## Research And History

- `docs/plan/18-*` and `docs/plan/19-*`: compact authority-store design and pre-store risk history
- `docs/plan/21-*` through `docs/plan/26-*`: terminal-agent bridge and capability research
- `docs/plan/27-*`: runtime `:task` intake design
- `docs/plan/28-*` and `docs/plan/29-*`: terminal rendering research and test methodology
- `docs/plan/30-*`: jcode competitor code deep dive
- [training/README.md](training/README.md): Rust training material using this repo as curriculum

## Rules

- Keep implemented behavior in `docs/supersession/current-contract.md`.
- Keep unfinished work in `docs/supersession/remaining-work.md`.
- Keep design rules in `docs/design/` and sequencing in `docs/plan/17-*` or `docs/plan/20-*`.
- Do not promote research notes into active roadmap unless they name a next implementation slice.
- Prefer links to current truth over repeating the same contract in multiple places.
