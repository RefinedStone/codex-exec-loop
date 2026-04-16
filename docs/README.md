# Native Docs

Current-state docs only. Keep contracts, entrypoints, and constraints. Remove rollout history, future-state prose, and "how to implement" notes once code ships.

## Read This First

1. [design/01-current-product-state.md](design/01-current-product-state.md)
2. [design/02-tui-shell-flow.md](design/02-tui-shell-flow.md)
3. [design/06-planning-runtime-and-draft-editor.md](design/06-planning-runtime-and-draft-editor.md)
4. [design/04-hexagonal-runtime-architecture.md](design/04-hexagonal-runtime-architecture.md)

## Open When Relevant

- [plan/10-inline-scrollback-shell.md](plan/10-inline-scrollback-shell.md): inline shell contract
- [plan/04-worktree-branch-rules.md](plan/04-worktree-branch-rules.md): branch and worktree rules
- [plan/11-parallel-worktree-plan.md](plan/11-parallel-worktree-plan.md): current concurrent slices
- [plan/12-platform-validation-matrix.md](plan/12-platform-validation-matrix.md): manual terminal validation
- [plan/13-native-packaging-and-operator-runbook.md](plan/13-native-packaging-and-operator-runbook.md): release and handoff
- [validation/README.md](validation/README.md): validation artifact naming

## Doc Rules

- One topic, one contract doc.
- Prefer current behavior and invariants over design background.
- Link to code entrypoints instead of repeating implementation steps.
- Delete superseded feature notes instead of accumulating archives in `docs/design/`.
