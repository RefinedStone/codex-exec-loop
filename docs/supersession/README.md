# Supersession Planning

This document set defines the target supersession model, not shipped behavior.

## Purpose

`docs/supersession/` is the decision-complete planning lane for the proposed parallel mode.
It treats the feature as a product and runtime redesign, not as a small extension of the
current hidden planning worker or session browser.

Current shipped supersession behavior should be read from `docs/design/` first.
This directory now exists to track the remaining target model and architecture follow-through.
Most major supersession slices are already implemented on the current branch, so treat this directory as residual follow-through and historical design context rather than the primary source of current behavior.

## Current Shipped Status

Current `prerelease` already ships:

- parallel-mode readiness gating and supersession control-tower entry
- a live supervisor board with pool, roster, selected detail, and distributor projections
- a three-slot `akra` worktree pool with lease lifecycle and cleanup
- `reported_complete` versus official completion through hidden planning refresh
- a serial distributor queue with GitHub automation and slot return

Current branch follow-through also includes:

- canonical repo-scoped authority location and shadow-store parity inspection
- store-backed drafts, validation, promote, and rollback-safe activation
- store-backed active planning mutation plus official refresh and distributor queue claims
- store-backed runtime projections and restart recovery classification
- store-primary active planning reads and export repair, plus legacy bootstrap cleanup

Use these docs for current behavior:

- [../design/01-current-product-state.md](../design/01-current-product-state.md)
- [../design/06-planning-runtime-and-draft-editor.md](../design/06-planning-runtime-and-draft-editor.md)
- [../releases/v1.2.9-to-prerelease.md](../releases/v1.2.9-to-prerelease.md)

Branch-local slice completion status also lives in:

- `./.codex-exec-loop/planning/directions.toml`
- `./.codex-exec-loop/planning/task-ledger.json`

## Fixed V1 Decisions

- parallel mode runs only in a git repository
- the integration branch is `akra`
- the worktree pool default size is `3`
- the execution unit is a main-grade agent session, not a planning worker
- the supervisor is a control tower and does not act as an implementation chat surface
- active task authority lives in a repo-scoped planning authority domain; tracked planning files are review and portability artifacts
- hidden planning worker refresh remains in the loop after agent completion
- distributor processes merge queue items one at a time
- GitHub automation uses `gh` capability when available and reports degraded readiness otherwise

## Recommended Reading Order

1. [01-product-model.md](01-product-model.md)
2. [02-operator-mode-and-shell-model.md](02-operator-mode-and-shell-model.md)
3. [03-agent-session-lifecycle.md](03-agent-session-lifecycle.md)
4. [04-task-ledger-feedback-loop.md](04-task-ledger-feedback-loop.md)
5. [05-git-worktree-pool.md](05-git-worktree-pool.md)
6. [06-distributor-and-merge-queue.md](06-distributor-and-merge-queue.md)
7. [07-supervisor-ui-and-surfaces.md](07-supervisor-ui-and-surfaces.md)
8. [09-architecture-boundaries.md](09-architecture-boundaries.md)
9. [08-capabilities-degraded-mode-and-failures.md](08-capabilities-degraded-mode-and-failures.md)
10. [10-implementation-slices.md](10-implementation-slices.md)
11. [11-open-questions-and-non-goals.md](11-open-questions-and-non-goals.md)

## File Map

| File | Role | Main output |
| --- | --- | --- |
| `01-product-model.md` | product and concept model | defines supersession, agents, pool, distributor, and ledger roles |
| `02-operator-mode-and-shell-model.md` | operator-visible IA | defines mode switching, overlay ownership, and shell expectations |
| `03-agent-session-lifecycle.md` | runtime lifecycle | defines agent state transitions and completion contract |
| `04-task-ledger-feedback-loop.md` | planning authority | defines how agent results become official active task state |
| `05-git-worktree-pool.md` | git orchestration | defines slot, worktree, branch, cleanup, and exhaustion rules |
| `06-distributor-and-merge-queue.md` | integration flow | defines push, PR, merge, cleanup, and serial distributor behavior |
| `07-supervisor-ui-and-surfaces.md` | UI contract | defines supersession surface, panels, summaries, and alerts |
| `08-capabilities-degraded-mode-and-failures.md` | readiness and recovery | defines capability gates and failure-state operator guidance |
| `09-architecture-boundaries.md` | code architecture | defines new ports, snapshots, adapters, and runtime splits |
| `10-implementation-slices.md` | delivery plan | defines slice order, validation, and worktree-safe boundaries |
| `11-open-questions-and-non-goals.md` | future boundary | records deferred design, non-goals, and later expansion points |

## Remaining Follow-Through

- real-terminal validation depth for restart and recovery paths
- compact docs alignment now that most architecture slices are implemented
- residual copy and surface polish instead of new authority or distributor subsystems

## Historical Handoff Summary

- Build a separate supersession runtime instead of stretching the current single conversation runtime.
- Reuse the hidden planning worker only for ledger refresh after agent milestones.
- Keep normal mode intact and route `:sessions` to supersession only when parallel mode is on.
- Treat git/worktree orchestration as a first-class outbound boundary, not as shell-script glue.
- Treat merge queue and distributor as a separate subsystem from agent execution.
- Preserve `draft -> validate -> promote` while moving planning authority into a repo-scoped store.

## Related Docs

- [../design/01-current-product-state.md](../design/01-current-product-state.md)
- [../design/02-tui-shell-flow.md](../design/02-tui-shell-flow.md)
- [../design/06-planning-runtime-and-draft-editor.md](../design/06-planning-runtime-and-draft-editor.md)
- [../plan/04-worktree-branch-rules.md](../plan/04-worktree-branch-rules.md)
- [../plan/11-parallel-worktree-plan.md](../plan/11-parallel-worktree-plan.md)

## Code Impact

Expected entrypoints:

- `src/adapter/inbound/tui/app`
- `src/application/service/planning`
- `src/application/service/session_service.rs`
- `src/adapter/outbound/codex_app_server_adapter.rs`
