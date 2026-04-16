# Native Docs

This repository keeps current-state contracts and future work plans side by side, but they serve different purposes.

- `docs/design/`: current shipped behavior, entrypoints, invariants, and constraints
- `docs/plan/`: execution blueprints, operator runbooks, and pre-implementation planning
- `docs/validation/`: captured validation artifacts and naming rules

## Read This First

1. [design/01-current-product-state.md](design/01-current-product-state.md)
2. [design/02-tui-shell-flow.md](design/02-tui-shell-flow.md)
3. [design/06-planning-runtime-and-draft-editor.md](design/06-planning-runtime-and-draft-editor.md)
4. [design/04-hexagonal-runtime-architecture.md](design/04-hexagonal-runtime-architecture.md)

## Open When Shaping The Next Iteration

- [plan/14-product-elevation-blueprint.md](plan/14-product-elevation-blueprint.md): product-level execution blueprint for the next stage
- [plan/15-ux-flow-rearchitecture.md](plan/15-ux-flow-rearchitecture.md): operator flow and status-language redesign
- [plan/16-planning-and-automation-evolution.md](plan/16-planning-and-automation-evolution.md): planning, queue, and automation evolution path
- [plan/17-structure-and-architecture-debt-map.md](plan/17-structure-and-architecture-debt-map.md): structural debt map tied to operator impact

## Open When Relevant

- [plan/10-inline-scrollback-shell.md](plan/10-inline-scrollback-shell.md): inline shell contract
- [plan/04-worktree-branch-rules.md](plan/04-worktree-branch-rules.md): branch and worktree rules
- [plan/11-parallel-worktree-plan.md](plan/11-parallel-worktree-plan.md): current concurrent slices
- [plan/12-platform-validation-matrix.md](plan/12-platform-validation-matrix.md): manual terminal validation
- [plan/13-native-packaging-and-operator-runbook.md](plan/13-native-packaging-and-operator-runbook.md): release and handoff
- [validation/README.md](validation/README.md): validation artifact naming

## Doc Rules

- `docs/design/` records shipped behavior only.
- `docs/plan/` records future-state choices, rollout order, and acceptance criteria until they ship.
- When plan content ships, move the durable contract back into `docs/design/` and delete superseded plan prose.
- One topic, one doc.
- Link to code entrypoints instead of repeating implementation steps.
- Delete superseded feature notes instead of accumulating archives.
