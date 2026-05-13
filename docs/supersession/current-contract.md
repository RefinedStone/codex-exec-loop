# Current Supersession And Planning Contract

This is the current operator-facing contract. Keep implemented behavior here and keep future
implementation planning out of this file.

## Snapshot

- One inline shell carries startup diagnostics, session resume, planning state, queue state,
  post-turn continuation, and supersession supervision.
- Accepted planning authority is DB-backed. Tracked planning files are review/export/staged-edit
  artifacts, not the runtime source of truth for git-backed workspaces.
- Supersession is shipped for git-backed workspaces with a fixed local worktree pool and serialized
  distributor lane.

## Operator Surfaces

| Surface | Entry | Contract |
| --- | --- | --- |
| Diagnostics | `Ctrl+d`, `:diag` | inspect startup readiness and blocking failures |
| Sessions | `Ctrl+o`, `:sessions` | resume prior threads with current workspace context |
| Queue | `:queue`, `:q`, `akra queue` | inspect accepted queue head, proposals, and skip framing |
| Planning Health | `:doctor`, `:planning doctor`, `akra doctor`, `akra status` | inspect planning state without authoring |
| Planning | `:planning`, `:planning-init` | stage or reopen planning authoring |
| Directions | `:directions` | maintain direction docs and queue-idle supporting prompt |
| Task Intake | Admin/API | add one validated user task as accepted `ready` work |
| Structured Tool | `akra planning-tool contract`, `akra planning-tool run` | expose and apply bounded planning task mutations for automation callers |
| Supersession | `:parallel`, `:pa` | arm parallel automation and refresh the supervisor board/pool |
| Supersession Off | `:parallel off`, `:pa off` | stop local parallel mode and close the board without deleting worktrees |

## Shell Command Contract

- `:planning` opens the planning control center. `:planning doctor` runs the planning-health view.
- `:directions` opens direction-side planning maintenance without accepting extra arguments.
- `:reset queue` immediately resets queue-side planning state.
- `:reset directions` and `:reset all` first render preview guidance; rerun with `confirm` to apply.
- `:model` opens model and reasoning-effort selection.
- `:think <none|minimal|low|medium|high|xhigh|default>` sets reasoning effort directly.
- `:turns <number|infinite>` sets the internal auto-follow turn budget.
- `:stop` stops active app-server sessions.
- `:help` lists the implemented command registry.

## Planning Contract

- Accepted planning still follows `draft -> validate -> promote`; direct active-state mutation is
  not the primary authoring path.
- Git-backed accepted task authority lives in SQLite task tables behind
  `PlanningTaskRepositoryPort`.
- Builtin `next-task` and internal continuation execute only the accepted queue head.
- Proposed tasks are visible but not executable until promoted or otherwise moved into normal queue
  state.
- Queue-idle behavior follows accepted DB direction authority.
- Admin task intake creates one validated accepted task. It does not interrupt an existing
  `in_progress` task.
- `akra planning-tool` is the structured automation boundary for list/create/update task mutations.

## Supersession Contract

- Bare `:parallel`/`:pa` is the enable/refresh entrypoint; `:parallel on` is not implemented.
- Off-to-on `:parallel`/`:pa` entry checks readiness, opens the board, attempts a pool-only reset and
  reconcile, opens an automation epoch, and dispatches any already-ready accepted queue up to
  idle-slot capacity.
- The first off-to-on `:parallel` in a TUI process treats the pool as disposable initial setup:
  registered `akra` slots are forced back to the current `prerelease` baseline and stale lease,
  session, and distributor mirrors for reset slots are cleared.
- Re-running `:parallel`/`:pa` while already enabled refreshes readiness and supervisor projection
  only; it does not reset the pool, reopen the automation epoch, or launch workers by itself.
- `Esc` closes the board surface only. Parallel mode remains enabled.
- `:parallel off`/`:pa off` disables local parallel mode and clears the automation epoch, pending
  dispatch, and in-flight dispatch state, but leaves pool worktrees in place.
- Later off-to-on `:parallel` entries attempt a guarded reset. Reset is blocked when live Running,
  CleanupPending, or recent Leased slots are present.
- Idle or stale reusable slots are reset into disposable baselines; protected active slots are
  preserved only after the initial setup reset has completed once in the process.
- Parallel automation starts when `:parallel`/`:pa` opens an automation epoch with an accepted ready
  queue, or after the main session completes a user turn and post-turn planning evaluation returns
  an accepted ready queue.
- In the post-turn case the normal main-session auto-follow prompt is suppressed and converted into
  parallel dispatch.
- Admin/manual task intake before successful `:parallel` entry commits accepted work only; it records
  a dispatch-withheld reason and launches no worker. After the epoch is open, task intake can request
  dispatch with the `task_intake_after_epoch` trigger.
- Dispatch triggers are explicit: `main_turn_post_evaluation`, `parallel_official_completion`, and
  `task_intake_after_epoch`.
- Concurrent requests coalesce into one in-flight dispatch pass plus at most one pending follow-up
  pass.
- The board shows readiness, pool, roster, selected detail, distributor head, and queue state.
- Selected detail includes a read-only compact lifecycle timeline for the selected session and keeps
  full history below as audit context.
- When distributor history exists, selected detail also shows a read-only delivery boundary row for
  source push, PR automation, and merge/integration timing.
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

- Store-backed claims coordinate official refresh and distributor queue-head processing.
- Stale official refresh recovery may abandon only the current head order per pass.
- Retryable distributor push recovery is limited to source branch push failures.
- Integration branch push blocks remain operator-owned.
- Failed-start dispatch blocks survive pool reset per task, keeping the latest `blocked_at`.
- Stale startup leases require matching session-detail evidence before automatic cleanup.

## Current Limits

- Real-terminal validation remains required for restart, blocked distributor, and multi-worktree
  operator flows.
- Planning detail mode remains manual; `llm-assisted` authoring is disabled.
- Approval approve/deny UI is gated by app-server capability and is not exposed in the current TUI.
- Non-git workspaces do not use the full supersession worktree pool model.

## Code Entry

- Core app runtime: `src/core/`
- Shell runtime: `src/adapter/inbound/tui/app.rs`
- Shell command registry: `src/adapter/inbound/tui/app/inline_shell_commands.rs`
- Supersession shell entrypoint: `src/adapter/inbound/tui/app/parallel_mode.rs`
- Supersession control-plane: `src/application/service/parallel_mode/control_plane/`
- Supersession application services: `src/application/service/parallel_mode/`
- Planning feature entrypoint: `src/adapter/inbound/tui/app/planning/`
- Planning services: `src/application/service/planning/`
- Planning authority adapter: `src/adapter/outbound/db/sqlite_planning_authority_adapter.rs`
