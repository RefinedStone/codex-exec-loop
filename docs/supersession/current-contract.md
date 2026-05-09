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
| Supersession | `:parallel` | arm parallel automation and refresh the supervisor board/pool |
| Supersession Off | `:parallel off` | stop local parallel mode and close the board without deleting worktrees |

## Planning Contract

- `:doctor` / `akra doctor`: read-only planning inspection.
- `:planning`: create or stage the default planning scaffold.
- `:reset queue|directions|all`: rewrite the selected accepted planning scope.
- `:planning on|off`: toggle plan execution without deleting the workspace.
- `:task [prompt]`: create a structured task draft, validate it, then commit one accepted task.
- Builtin `next-task` and internal continuation execute only the accepted queue head.
- Proposed tasks are visible but not executable until promoted.
- Queue-idle behavior follows accepted DB direction authority.

## Supersession Contract

- Bare `:parallel` is the enable/refresh entrypoint; `:parallel on` is not a separate mode.
- Off-to-on `:parallel` entry checks readiness, opens the board, attempts a pool-only reset and
  reconcile, opens an automation epoch, and dispatches any already-ready accepted queue up to
  idle-slot capacity.
- The first off-to-on `:parallel` in a TUI process treats the pool as disposable initial setup:
  every registered `akra` slot is forced back to the current `prerelease` baseline and stale lease,
  session, and distributor mirrors for reset slots are cleared.
- Re-running `:parallel` while already enabled refreshes readiness and supervisor projection only;
  it does not reset the pool, reopen the automation epoch, or launch workers by itself.
- `Esc` closes the board surface only. Parallel mode remains enabled.
- `:parallel off` disables local parallel mode and clears the automation epoch, pending dispatch,
  and in-flight dispatch state, but leaves pool worktrees in place.
- Later off-to-on `:parallel` entries attempt a guarded reset. Reset is blocked when live Running,
  CleanupPending, or recent Leased slots are present.
- Idle or stale reusable slots are reset into disposable baselines; protected active slots are
  preserved only after the initial setup reset has completed once in the process.
- Parallel automation starts when `:parallel` opens an automation epoch with an accepted ready
  queue, or after the main session completes a user turn and post-turn planning evaluation returns
  an accepted ready queue. In the post-turn case the normal main-session auto-follow prompt is
  suppressed and converted into parallel dispatch.
- Manual `:task` intake before successful `:parallel` entry commits accepted work only; it records a
  dispatch-withheld reason and launches no worker. After the epoch is open, task intake can request
  dispatch with the `task_intake_after_epoch` trigger.
- Dispatch triggers are explicit: `main_turn_post_evaluation`, `parallel_official_completion`, and
  `task_intake_after_epoch`. Concurrent requests coalesce into one in-flight dispatch pass plus at
  most one pending follow-up pass.
- The board shows readiness, pool, roster, selected detail, distributor head, and queue state.
- Selected detail includes a read-only compact lifecycle timeline for the selected session:
  timestamped history states are condensed into event arrows, with full history kept below as
  audit context.
- When distributor history exists, selected detail also shows a read-only delivery boundary row
  for source push, PR automation, and merge/integration timing so those handoffs stay visible after
  the compact lifecycle condenses older events.
- The board summary includes the last automation trigger and the latest dispatch-withheld reason
  when either exists.
- Queue work leases one of three local `akra` worktree slots.
- Agent completion becomes distributor-eligible only after hidden official planning refresh marks it
  commit-ready.
- A successful parallel official completion refresh that leaves another actionable queue head emits
  the next `parallel_official_completion` dispatch request, capped by idle-slot capacity.
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
