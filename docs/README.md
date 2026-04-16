# Docs Map

The repository keeps shipped contracts, release deltas, validation records, and roadmap docs in separate lanes.

- `docs/design/`: current shipped behavior and code-entry contracts
- `docs/releases/`: curated change summaries between notable tags and the current product
- `docs/validation/`: captured terminal validation records and naming rules
- `docs/plan/`: operator runbooks plus roadmap and architecture references
- `docs/supersession/`: target parallel-mode and supersession architecture planning
- `docs/agent/`: contributor guidance for repo-specific workflows

## Start Here

1. [design/01-current-product-state.md](design/01-current-product-state.md)
2. [design/02-tui-shell-flow.md](design/02-tui-shell-flow.md)
3. [design/06-planning-runtime-and-draft-editor.md](design/06-planning-runtime-and-draft-editor.md)
4. [releases/v1.2.9-to-prerelease.md](releases/v1.2.9-to-prerelease.md)

## Operator Docs

- [plan/13-native-packaging-and-operator-runbook.md](plan/13-native-packaging-and-operator-runbook.md): packaging, release, and operator handoff
- [plan/12-platform-validation-matrix.md](plan/12-platform-validation-matrix.md): validation expectations and check profiles
- [validation/README.md](validation/README.md): validation artifact naming and helper usage

## Architecture And Runtime

- [design/04-hexagonal-runtime-architecture.md](design/04-hexagonal-runtime-architecture.md)
- [plan/10-inline-scrollback-shell.md](plan/10-inline-scrollback-shell.md)
- [plan/17-structure-and-architecture-debt-map.md](plan/17-structure-and-architecture-debt-map.md)

## Roadmap References

- [plan/14-product-elevation-blueprint.md](plan/14-product-elevation-blueprint.md)
- [plan/15-ux-flow-rearchitecture.md](plan/15-ux-flow-rearchitecture.md)
- [plan/16-planning-and-automation-evolution.md](plan/16-planning-and-automation-evolution.md)

## Supersession Planning

- [supersession/README.md](supersession/README.md)

## Repo Workflow References

- [plan/04-worktree-branch-rules.md](plan/04-worktree-branch-rules.md)
- [plan/11-parallel-worktree-plan.md](plan/11-parallel-worktree-plan.md)
- [agent/README.md](agent/README.md)

## Doc Rules

- Keep shipped behavior in `docs/design/`.
- Keep release-delta summaries in `docs/releases/`.
- Keep runbooks and active roadmap work in `docs/plan/`.
- Keep target supersession and parallel-mode architecture planning in `docs/supersession/` until contracts are shipped.
- Delete superseded plan docs once the shipped contract has moved back into `docs/design/`.
- Prefer linking to code entrypoints instead of embedding long implementation walkthroughs.
