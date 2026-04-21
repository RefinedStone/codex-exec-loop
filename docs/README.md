# Docs Map

Use this map to answer three questions quickly: what ships now, what the active roadmap is, and which docs are history only.

## Architecture Principles

- `docs/design/04-hexagonal-runtime-architecture.md` is the source of truth for boundary rules.
- Small-context readability is a first-class design constraint for both humans and LLM-guided edits.
- Infrastructure-heavy implementations should sit behind ports and infra-specific directories so feature flow analysis can skip them.
- When behavior, rendering, storage, and recovery logic pile into one file, prefer boundary-first splits over local convenience.

## Current Product

- [design/01-current-product-state.md](design/01-current-product-state.md): current product and supersession status
- [design/02-tui-shell-flow.md](design/02-tui-shell-flow.md): operator-visible shell flow
- [design/06-planning-runtime-and-draft-editor.md](design/06-planning-runtime-and-draft-editor.md): current planning contract

## Active Roadmap

- [plan/20-context-first-architecture-and-doc-coherence.md](plan/20-context-first-architecture-and-doc-coherence.md): current short and mid-term plan for boundary cleanup, vocabulary alignment, and LLM-friendly structure
- [plan/21-terminal-agent-bridge-research-and-capability-boundary.md](plan/21-terminal-agent-bridge-research-and-capability-boundary.md): current short and mid-term plan for Claude Code class terminal-agent research
- [plan/17-structure-and-architecture-debt-map.md](plan/17-structure-and-architecture-debt-map.md): supporting hotspot map, current hotspot order, and refactor ordering input

## Release Delta

- [releases/v1.2.9-to-prerelease.md](releases/v1.2.9-to-prerelease.md): what `origin/prerelease` ships beyond `v1.2.9`

## Historical Context

- [supersession/README.md](supersession/README.md): supersession docs index
- [supersession/implemented-summary.md](supersession/implemented-summary.md): merged summary of shipped supersession contracts
- [supersession/10-implementation-slices.md](supersession/10-implementation-slices.md): remaining supersession follow-through notes
- [supersession/11-open-questions-and-non-goals.md](supersession/11-open-questions-and-non-goals.md): supersession-era open questions and non-goals

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
- [plan/18-repo-shared-planning-authority-store.md](plan/18-repo-shared-planning-authority-store.md)
- [plan/19-supersession-runtime-risk-audit.md](plan/19-supersession-runtime-risk-audit.md)

## Rules

- `docs/design/` holds current contracts.
- `docs/releases/` holds tagged or branch release deltas only.
- `docs/plan/20-*` and `docs/plan/21-*` hold the active roadmap for the current cycle.
- `docs/supersession/` holds merged history and follow-through context, not the active roadmap.
- `docs/plan/` holds runbooks, active roadmap, and historical design audits.
- Use `docs/plan/20-*` with `docs/plan/17-*` when a task touches a named hotspot; the roadmap explains why and the debt map fixes the current order.
- Prefer links to current truth over repeating the same contract in multiple places.
- Prefer flow-local documents over repo-spanning narrative; one question should usually require one primary doc plus at most one supporting link.
