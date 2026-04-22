# Docs Map

Use this map to answer four questions quickly: what ships now, what is still unfinished, which
docs are deep dives, and which docs are history only.

## Read First

- [supersession/README.md](supersession/README.md): canonical docs hub for supersession, planning, and directions
- [supersession/current-contract.md](supersession/current-contract.md): current shipped contract
- [supersession/remaining-work.md](supersession/remaining-work.md): unfinished or lightly validated work

## Architecture Principles

- `docs/design/04-hexagonal-runtime-architecture.md` is the source of truth for boundary rules.
- Small-context readability is a first-class design constraint for both humans and LLM-guided edits.
- Infrastructure-heavy implementations should sit behind ports and infra-specific directories so feature flow analysis can skip them.
- When behavior, rendering, storage, and recovery logic pile into one file, prefer boundary-first splits over local convenience.

## Supporting Deep Dives

- [design/01-current-product-state.md](design/01-current-product-state.md): product identity and surface map
- [design/02-tui-shell-flow.md](design/02-tui-shell-flow.md): operator-visible shell flow
- [design/06-planning-runtime-and-draft-editor.md](design/06-planning-runtime-and-draft-editor.md): planning/runtime implementation detail

## Active Roadmap

- [plan/20-context-first-architecture-and-doc-coherence.md](plan/20-context-first-architecture-and-doc-coherence.md): current short and mid-term plan for boundary cleanup, vocabulary alignment, and LLM-friendly structure
- [plan/21-terminal-agent-bridge-research-and-capability-boundary.md](plan/21-terminal-agent-bridge-research-and-capability-boundary.md): current short and mid-term plan for Claude Code class terminal-agent research
- [plan/17-structure-and-architecture-debt-map.md](plan/17-structure-and-architecture-debt-map.md): supporting hotspot map, current hotspot order, and refactor ordering input

## Release Delta

- [releases/v1.2.9-to-prerelease.md](releases/v1.2.9-to-prerelease.md): what `origin/prerelease` ships beyond `v1.2.9`

## Runbooks And Validation

- [plan/12-platform-validation-matrix.md](plan/12-platform-validation-matrix.md)
- [plan/13-native-packaging-and-operator-runbook.md](plan/13-native-packaging-and-operator-runbook.md)
- [validation/README.md](validation/README.md)
- [plan/04-worktree-branch-rules.md](plan/04-worktree-branch-rules.md)
- [plan/11-parallel-worktree-plan.md](plan/11-parallel-worktree-plan.md)

## Background References

- [plan/14-product-elevation-blueprint.md](plan/14-product-elevation-blueprint.md)
- [plan/15-ux-flow-rearchitecture.md](plan/15-ux-flow-rearchitecture.md)
- [plan/16-planning-and-automation-evolution.md](plan/16-planning-and-automation-evolution.md)
- [plan/18-repo-shared-planning-authority-store.md](plan/18-repo-shared-planning-authority-store.md): historical redesign record
- [plan/19-supersession-runtime-risk-audit.md](plan/19-supersession-runtime-risk-audit.md): historical pre-store risk audit

## Training Material

- [training/README.md](training/README.md): Spring Boot/Kotlin server developers를 위한 Rust 교본 인덱스
- [training/quality-baseline.md](training/quality-baseline.md): 강의 시작 전에 다시 측정할 품질 기준선

## Rules

- `docs/supersession/current-contract.md` is the canonical current contract for supersession,
  planning, and directions behavior.
- `docs/supersession/remaining-work.md` tracks unfinished, lightly validated, or intentionally
  deferred work.
- `docs/design/` holds deep technical explanation and boundary rules.
- `docs/releases/` holds tagged or branch release deltas only.
- `docs/plan/20-*` and `docs/plan/21-*` hold the active roadmap for the current cycle.
- `docs/supersession/` is the current-contract hub, not a historical side archive.
- `docs/plan/` holds runbooks, active roadmap, and historical design audits.
- Use `docs/plan/20-*` with `docs/plan/17-*` when a task touches a named hotspot; the roadmap explains why and the debt map fixes the current order.
- `docs/training/` holds lecture and curriculum material that uses the repo as a textbook.
- Prefer links to current truth over repeating the same contract in multiple places.
- Prefer flow-local documents over repo-spanning narrative; one question should usually require one primary doc plus at most one supporting link.
