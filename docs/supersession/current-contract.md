# Current Supersession And Planning Contract

This is the current operator-facing contract. Keep implemented behavior here; move unfinished work
to [remaining-work.md](remaining-work.md).

## Snapshot

- One inline shell carries startup diagnostics, session resume, planning state, queue state,
  post-turn continuation, task intake, and supersession supervision.
- Accepted planning authority is DB-backed. Tracked planning files are review/export/staged-edit
  artifacts, not the runtime source of truth.
- Supersession is shipped for git-backed workspaces with a fixed local worktree pool and serialized
  distributor lane.

## Operator Surfaces

| Surface | Entry | Contract |
| --- | --- | --- |
| Diagnostics | `Ctrl+d`, `:diag` | inspect startup readiness and blocking failures |
| Sessions | `Ctrl+o`, `:sessions` | resume prior threads with current workspace context |
| Queue | `:queue` | inspect accepted queue head, proposals, and skip framing |
| Task Intake | `:task` | preview and commit one validated user task as accepted `ready` work |
| Planning | `:planning` | stage or reopen planning authoring |
| Directions | `:directions` | maintain direction docs and queue-idle supporting prompt |
| Supersession | `:parallel` | enable or refresh the supervisor board and worker pool |
| Supersession Off | `:parallel off` | stop local parallel mode and close the board without deleting worktrees |

## Planning Contract

- `:doctor` / `akra doctor`: read-only planning inspection.
- `:init` / `akra init`: create or stage the default planning scaffold.
- `:reset queue|directions|all`: rewrite the selected accepted planning scope.
- `:planning on|off`: toggle plan execution without deleting the workspace.
- `:task [prompt]`: create a structured task draft, validate it, then commit one accepted task.
- Builtin `next-task` and internal continuation execute only the accepted queue head.
- Proposed tasks are visible but not executable until promoted.
- Queue-idle behavior follows accepted DB direction authority.

## Supersession Contract

- Bare `:parallel` is the enable/refresh entrypoint; `:parallel on` is not a separate mode.
- Off-to-on `:parallel` entry checks readiness, opens the board, and attempts pool reset.
- Re-running `:parallel` while already enabled refreshes readiness/reconcile/dispatch only; it does
  not reset the pool.
- `Esc` closes the board surface only. Parallel mode remains enabled.
- `:parallel off` disables local parallel mode and clears pending deferred dispatch, but leaves pool
  worktrees in place.
- The next off-to-on `:parallel` attempts reset again. Reset is blocked when live Running,
  CleanupPending, or recent Leased slots are present.
- Idle or stale reusable slots are reset into disposable baselines; active slots are preserved.
- Task intake while parallel mode is enabled triggers dispatch. If entry is still loading, dispatch
  is deferred until the concrete entry snapshot lands.
- The board shows readiness, pool, roster, selected detail, distributor head, and queue state.
- Queue work leases one of three local `akra` worktree slots.
- Agent completion becomes distributor-eligible only after hidden official planning refresh marks it
  commit-ready.
- Distributor delivery is serial: source branch push, PR automation, integration into `prerelease`,
  and slot cleanup.

## Recovery Contract

- Store-backed claims coordinate official refresh and distributor queue head processing.
- Stale official refresh recovery may abandon only the current head order per pass.
- Retryable distributor push recovery is limited to source branch push failures.
- Integration branch push blocks remain operator-owned.
- Failed-start dispatch blocks survive pool reset per task, keeping the latest `blocked_at`.
- Stale startup leases require matching session-detail evidence before automatic cleanup.

## Current Limits

- Real-terminal validation is still required for restart, blocked distributor, and multi-worktree
  operator flows.
- Planning detail mode remains manual; `llm-assisted` authoring is disabled.
- Approval approve/deny UI is still gated by available app-server capability.
- Non-git workspaces do not use the full supersession worktree pool model.

## Deep Dives

- [../design/01-current-product-state.md](../design/01-current-product-state.md)
- [../design/02-tui-shell-flow.md](../design/02-tui-shell-flow.md)
- [../design/06-planning-runtime-and-draft-editor.md](../design/06-planning-runtime-and-draft-editor.md)
- [remaining-work.md](remaining-work.md)
