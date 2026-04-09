# Native Docs

This folder is the compact current-state record for the Rust native client on `prerelease`.

## Reading Order
1. Read [design/01-current-product-state.md](design/01-current-product-state.md).
2. Read [design/02-tui-shell-flow.md](design/02-tui-shell-flow.md).
3. Read [design/04-hexagonal-runtime-architecture.md](design/04-hexagonal-runtime-architecture.md).
4. Read [design/03-auto-followup-and-templates.md](design/03-auto-followup-and-templates.md).
5. Read [plan/10-inline-scrollback-shell.md](plan/10-inline-scrollback-shell.md) for the current inline-shell contract.
6. Read [design/05-known-gaps-and-risk-areas.md](design/05-known-gaps-and-risk-areas.md) for durable constraints and maintenance risks.
7. Read [plan/04-worktree-branch-rules.md](plan/04-worktree-branch-rules.md) and [plan/11-parallel-worktree-plan.md](plan/11-parallel-worktree-plan.md) before splitting work across multiple git worktrees.
8. Read [plan/12-platform-validation-matrix.md](plan/12-platform-validation-matrix.md) when a PR changes terminal restore, frontend mode, or platform-facing shell behavior.
9. Use [validation/README.md](validation/README.md) when recording real macOS or Windows validation runs.

## Compaction Rule
- keep durable contracts, ownership boundaries, lifecycle notes, and operator-visible behavior
- remove stale sprint sequencing, completion logs, and old branch-by-branch history
- open a dedicated feature doc only when a new sprint or large workstream actually exists

## Document Map
- [design/00-main-to-prerelease-delta.md](design/00-main-to-prerelease-delta.md): short baseline delta from `main`
- [design/01-current-product-state.md](design/01-current-product-state.md): current product posture and shipped baseline
- [design/02-tui-shell-flow.md](design/02-tui-shell-flow.md): current shell shape and interaction model
- [design/03-auto-followup-and-templates.md](design/03-auto-followup-and-templates.md): core automation behavior
- [design/04-hexagonal-runtime-architecture.md](design/04-hexagonal-runtime-architecture.md): stable architectural boundaries and runtime lifecycle
- [design/05-known-gaps-and-risk-areas.md](design/05-known-gaps-and-risk-areas.md): durable constraints and maintenance risks worth preserving
- [design/06-event-driven-ui-poc.md](design/06-event-driven-ui-poc.md): reference design for a stricter future UI core
- [plan/01-roadmap.md](plan/01-roadmap.md): short statement of current product direction
- [plan/02-todo-backlog.md](plan/02-todo-backlog.md): reset marker for the next sprint backlog
- [plan/03-execution-order.md](plan/03-execution-order.md): current delivery posture
- [plan/04-worktree-branch-rules.md](plan/04-worktree-branch-rules.md): branch and worktree rules for concurrent native delivery
- [plan/10-inline-scrollback-shell.md](plan/10-inline-scrollback-shell.md): current inline-shell contract
- [plan/11-parallel-worktree-plan.md](plan/11-parallel-worktree-plan.md): current concurrency and hotspot snapshot
- [plan/12-platform-validation-matrix.md](plan/12-platform-validation-matrix.md): canonical manual validation matrix for macOS and Windows terminal behavior
- [validation/README.md](validation/README.md): canonical location and naming rules for checked-in validation result rows
