# Documentation Map

[한국어](ko/docs-map.md)

Use this page to select one source of truth. Current implementation references are compact and
bilingual. Executable validation contracts stay separate because repository tests parse their
markers and inventories. Future plans and competitor evidence remain unabridged.

## Current Implementation

| Need | Canonical document | Korean |
| --- | --- | --- |
| Product surfaces, commands, planning, parallel flow, limits | [reference/current-product.md](reference/current-product.md) | [ko/reference/current-product.md](ko/reference/current-product.md) |
| Layer ownership, state authority, security boundaries | [reference/architecture.md](reference/architecture.md) | [ko/reference/architecture.md](ko/reference/architecture.md) |
| Repository map, coding rules, tests, worktrees, GitHub delivery | [reference/development.md](reference/development.md) | [ko/reference/development.md](ko/reference/development.md) |
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

## Future and External Material

The following content is not compacted into current truth:

- [design/09-admin-game-control-center.md](design/09-admin-game-control-center.md): proposed Admin game control center and rollout plan
- [design/resources/admin-animation-map-concepts/](design/resources/admin-animation-map-concepts/): concept assets and prompts supporting that proposal
- [plan/14-codex-for-oss-application.md](plan/14-codex-for-oss-application.md): future program application draft with explicitly archival metrics
- [competitive/README.md](competitive/README.md): pinned competitor analyses, evidence ledgers, and gap matrices ([한국어](ko/competitive/README.md))

These documents may describe work that is not shipped. Their own status and evidence dates control
how they should be read.

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
- Do not compact future plans or competitor evidence into shipped behavior.
- Do not hand-edit generated validation status counts; run the summary helpers against real records.
