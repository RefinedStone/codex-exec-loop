# Native Docs

This folder is a compact snapshot of the Rust native client after the first development pass on `prerelease`.

The docs now optimize for phase-2 work:

- keep durable context around core logic, runtime boundaries, and automation rules
- keep completed baseline and shipped-shape docs compact
- allow concrete large refactor workstreams to carry detailed planning when that detail is useful

## Reading Order
1. Read [design/01-current-product-state.md](design/01-current-product-state.md).
2. Read [design/04-hexagonal-runtime-architecture.md](design/04-hexagonal-runtime-architecture.md).
3. Read [design/03-auto-followup-and-templates.md](design/03-auto-followup-and-templates.md).
4. If you are working on the remaining terminal-flow shell target, read [plan/10-inline-scrollback-shell.md](plan/10-inline-scrollback-shell.md) next.
5. Use [design/05-known-gaps-and-risk-areas.md](design/05-known-gaps-and-risk-areas.md), [plan/02-todo-backlog.md](plan/02-todo-backlog.md), and [plan/11-parallel-worktree-plan.md](plan/11-parallel-worktree-plan.md) for the current remaining-work baseline.
6. Read [plan/04-worktree-branch-rules.md](plan/04-worktree-branch-rules.md) and [plan/11-parallel-worktree-plan.md](plan/11-parallel-worktree-plan.md) before splitting active work across multiple git worktrees.
7. Read [plan/12-platform-validation-matrix.md](plan/12-platform-validation-matrix.md) when a PR changes terminal restore, frontend mode, or platform-facing shell behavior.
8. Use [validation/README.md](validation/README.md) when recording real macOS or Windows matrix runs.

## Compaction Rule
- Core logic docs should keep stable contracts, ownership boundaries, lifecycle notes, and stop-rule behavior.
- UI/UX docs for the current shipped baseline should describe the implemented form at a high level, not preserve every interaction detail.
- baseline `plan/` docs should stay short and describe the current planning posture.
- concrete future workstream docs under `plan/` may be detailed when the change is large enough that the detail reduces execution risk.

## Document Map
- [design/00-main-to-prerelease-delta.md](design/00-main-to-prerelease-delta.md): short baseline delta from `main`
- [design/01-current-product-state.md](design/01-current-product-state.md): shipped capabilities and current product posture
- [design/02-tui-shell-flow.md](design/02-tui-shell-flow.md): compact summary of the current shell shape
- [design/03-auto-followup-and-templates.md](design/03-auto-followup-and-templates.md): core automation behavior
- [design/04-hexagonal-runtime-architecture.md](design/04-hexagonal-runtime-architecture.md): stable architectural boundaries and runtime lifecycle
- [design/05-known-gaps-and-risk-areas.md](design/05-known-gaps-and-risk-areas.md): current risks worth preserving in context
- [design/06-event-driven-ui-poc.md](design/06-event-driven-ui-poc.md): event-driven reference for the final-stage strict UI core, not the current production baseline
- [plan/01-roadmap.md](plan/01-roadmap.md): current planning baseline, not a future milestone script
- [plan/02-todo-backlog.md](plan/02-todo-backlog.md): current open change buckets that still matter across PRs
- [plan/03-execution-order.md](plan/03-execution-order.md): current delivery posture and how future feature docs should take over detail
- [plan/04-worktree-branch-rules.md](plan/04-worktree-branch-rules.md): branch and worktree rules for concurrent native delivery
- [plan/10-inline-scrollback-shell.md](plan/10-inline-scrollback-shell.md): active reference doc for the remaining terminal-flow shell target and the `Transcript / tail` reset
- [plan/11-parallel-worktree-plan.md](plan/11-parallel-worktree-plan.md): compact completion snapshot plus detailed notes for the remaining terminal-flow and platform-validation slices
- [plan/12-platform-validation-matrix.md](plan/12-platform-validation-matrix.md): canonical manual validation matrix for macOS and Windows terminal behavior
- [validation/README.md](validation/README.md): canonical location and naming rules for checked-in validation result rows
