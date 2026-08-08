# Documentation Map

[한국어](ko/docs-map.md)

Use this page to select one source of truth. Current implementation references are compact and
bilingual. Executable validation contracts stay separate because repository tests parse their
markers and inventories. Future plans carry an explicit status; competitor research keeps one
current pinned brief per product.

## Current Implementation

| Need | Canonical document | Korean |
| --- | --- | --- |
| Product surfaces, commands, planning, parallel flow, limits | [reference/current-product.md](reference/current-product.md) | [ko/reference/current-product.md](ko/reference/current-product.md) |
| Layer ownership, state authority, security boundaries | [reference/architecture.md](reference/architecture.md) | [ko/reference/architecture.md](ko/reference/architecture.md) |
| Repository map, coding rules, tests, worktrees, GitHub delivery | [reference/development.md](reference/development.md) | [ko/reference/development.md](ko/reference/development.md) |
| Admin game frontend runtime and renderer | [reference/admin-game-frontend.md](reference/admin-game-frontend.md) | [Korean overview](ko/reference/current-product.md#admin-게임-프런트엔드) |
| Install and first run | [../README.md](../README.md) | [ko/README.md](ko/README.md) |
| Native packaging and publication | [plan/13-native-packaging-and-operator-runbook.md](plan/13-native-packaging-and-operator-runbook.md) | [ko/reference/release.md](ko/reference/release.md) |

## TUI and Validation Contracts

These files are intentionally separate from the narrative references. Architecture tests check
their exact headings, phrases, surface rows, or test-entrypoint inventory.

- [design/07-tui-layered-architecture-and-aesthetic-contract.md](design/07-tui-layered-architecture-and-aesthetic-contract.md): TUI layer and visual ownership ([한국어](ko/reference/tui-contract.md))
- [plan/12-platform-validation-matrix.md](plan/12-platform-validation-matrix.md): required platform rows and capture profiles ([한국어](ko/reference/validation.md))
- [validation/terminal-ui-testing-methodology.md](validation/terminal-ui-testing-methodology.md): automated and manual proof method ([한국어](ko/reference/validation.md))
- [validation/tui-coverage-matrix.md](validation/tui-coverage-matrix.md): code-mapped TUI test inventory ([한국어 안내](ko/reference/validation.md))
- [validation/README.md](validation/README.md): checked-in evidence and capture commands

## Plans and External Research

The following content is not implementation truth:

- [plan/14-codex-for-oss-application.md](plan/14-codex-for-oss-application.md): future program application draft with explicitly archival metrics
- [competitive/README.md](competitive/README.md): current compact competitive snapshots, methodology, and cache/token study ([한국어 요약](ko/competitive/README.md))

Their own status and evidence dates control how they should be read.

## Historical Evidence

- `docs/validation/*.txt` and [validation/artifacts/](validation/artifacts/) contain reviewable
  validation evidence.
- [../artifacts/terminal-bridge-readiness-2026-04-23/](../artifacts/terminal-bridge-readiness-2026-04-23/) is historical feasibility evidence, not a current required-row pass.

One-off audits, completed status reports, stale training baselines, and empty coordination plans do
not belong in this map. Use Git history when their historical reasoning is needed.

## Maintenance Rules

- Update one canonical current reference instead of copying the same contract into several files.
- Keep the English source and its `docs/ko/` translation linked in both directions.
- Update translations in the same PR when current reference meaning changes.
- Keep one canonical competitor brief per product and a compact Korean index; replace stale
  snapshots in place.
- Do not promote future plans or competitor conclusions into shipped behavior.
- Do not hand-edit generated validation status counts; run the summary helpers against real records.
