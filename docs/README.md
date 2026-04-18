# Docs Map

Use one source of truth per question.

## Current Product

- [design/01-current-product-state.md](design/01-current-product-state.md): current product and supersession status
- [design/02-tui-shell-flow.md](design/02-tui-shell-flow.md): operator-visible shell flow
- [design/06-planning-runtime-and-draft-editor.md](design/06-planning-runtime-and-draft-editor.md): current planning contract

## Release Delta

- [releases/v1.2.9-to-prerelease.md](releases/v1.2.9-to-prerelease.md): what `origin/prerelease` ships beyond `v1.2.9`

## Remaining Supersession Work

- [supersession/README.md](supersession/README.md): supersession docs index
- [supersession/implemented-summary.md](supersession/implemented-summary.md): merged summary of implemented supersession contracts
- [supersession/10-implementation-slices.md](supersession/10-implementation-slices.md): remaining validation, docs, and polish work
- [supersession/11-open-questions-and-non-goals.md](supersession/11-open-questions-and-non-goals.md): live open questions and explicit non-goals

## Runbooks And Validation

- [plan/12-platform-validation-matrix.md](plan/12-platform-validation-matrix.md)
- [plan/13-native-packaging-and-operator-runbook.md](plan/13-native-packaging-and-operator-runbook.md)
- [validation/README.md](validation/README.md)
- [plan/04-worktree-branch-rules.md](plan/04-worktree-branch-rules.md)
- [plan/11-parallel-worktree-plan.md](plan/11-parallel-worktree-plan.md)

## Historical And Roadmap References

- [plan/18-repo-shared-planning-authority-store.md](plan/18-repo-shared-planning-authority-store.md)
- [plan/19-supersession-runtime-risk-audit.md](plan/19-supersession-runtime-risk-audit.md)
- [plan/14-product-elevation-blueprint.md](plan/14-product-elevation-blueprint.md)
- [plan/15-ux-flow-rearchitecture.md](plan/15-ux-flow-rearchitecture.md)
- [plan/16-planning-and-automation-evolution.md](plan/16-planning-and-automation-evolution.md)
- [plan/17-structure-and-architecture-debt-map.md](plan/17-structure-and-architecture-debt-map.md)

## Rules

- `docs/design/` holds current contracts.
- `docs/releases/` holds tagged or branch release deltas only.
- `docs/supersession/` holds remaining work plus merged historical context, not the primary current truth.
- `docs/plan/` holds runbooks, roadmap, and historical design audits.
- Prefer links to current truth over repeating the same contract in multiple places.
