# Native Docs

This folder keeps only current-state docs for the Rust native client.

Do not keep completed rollout notes, stale backlogs, or future-state design prose here. Add a new feature note only while a live workstream actually needs one.

## Reading Order

1. [design/01-current-product-state.md](design/01-current-product-state.md)
2. [design/02-tui-shell-flow.md](design/02-tui-shell-flow.md)
3. [design/03-auto-followup-and-templates.md](design/03-auto-followup-and-templates.md)
4. [design/06-planning-runtime-and-draft-editor.md](design/06-planning-runtime-and-draft-editor.md)
5. [design/04-hexagonal-runtime-architecture.md](design/04-hexagonal-runtime-architecture.md)
6. [design/05-known-gaps-and-risk-areas.md](design/05-known-gaps-and-risk-areas.md)
7. [plan/10-inline-scrollback-shell.md](plan/10-inline-scrollback-shell.md)
8. [plan/04-worktree-branch-rules.md](plan/04-worktree-branch-rules.md) and [plan/11-parallel-worktree-plan.md](plan/11-parallel-worktree-plan.md) when parallel branches are involved
9. [plan/12-platform-validation-matrix.md](plan/12-platform-validation-matrix.md) when terminal behavior changes
10. [plan/13-native-packaging-and-operator-runbook.md](plan/13-native-packaging-and-operator-runbook.md) for packaging and handoff
11. [validation/README.md](validation/README.md) when recording a real validation run

## Document Map

- [design/01-current-product-state.md](design/01-current-product-state.md): shipped product baseline
- [design/02-tui-shell-flow.md](design/02-tui-shell-flow.md): shell layout, commands, and interaction model
- [design/03-auto-followup-and-templates.md](design/03-auto-followup-and-templates.md): auto-follow template catalog and runtime rules
- [design/04-hexagonal-runtime-architecture.md](design/04-hexagonal-runtime-architecture.md): stable layer ownership and runtime boundary
- [design/05-known-gaps-and-risk-areas.md](design/05-known-gaps-and-risk-areas.md): durable constraints worth preserving
- [design/06-planning-runtime-and-draft-editor.md](design/06-planning-runtime-and-draft-editor.md): current planning files, queue behavior, and draft-editor contract
- [plan/04-worktree-branch-rules.md](plan/04-worktree-branch-rules.md): branch naming, worktree, and linear merge rules
- [plan/10-inline-scrollback-shell.md](plan/10-inline-scrollback-shell.md): inline shell contract
- [plan/11-parallel-worktree-plan.md](plan/11-parallel-worktree-plan.md): lightweight live snapshot for concurrent work
- [plan/12-platform-validation-matrix.md](plan/12-platform-validation-matrix.md): manual terminal validation matrix
- [plan/13-native-packaging-and-operator-runbook.md](plan/13-native-packaging-and-operator-runbook.md): release bundle and operator handoff
- [validation/README.md](validation/README.md): validation record naming and helper usage
