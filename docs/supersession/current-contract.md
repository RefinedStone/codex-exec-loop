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
- `:turns <number|infinite>` sets the single-session auto-follow turn budget. It is independent of
  parallel automation.
- `:stop` stops active app-server sessions, closes the active parallel epoch, and disarms both
  continuation paths. A later `:parallel` re-arms only parallel continuation; a positive or infinite
  `:turns` command is still required to re-arm single-session auto-follow.
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
- Parallel automation is an independent explicit opt-in and does not require `:turns`. After
  `:stop`, off-to-on `:parallel` re-arms only parallel continuation and preserves the sticky
  single-session stop.
- Off-to-on `:parallel`/`:pa` entry checks readiness, opens the board, attempts a pool-only reset and
  reconcile, opens an automation epoch, and dispatches any already-ready accepted queue up to
  idle-slot capacity.
- The first off-to-on `:parallel` in a TUI process may clear disposable runtime projections only
  after every affected slot is proven clean and resettable. Dirty, untracked, pending-operation,
  unleased non-baseline, and non-integrated slots are preserved and block projection-wide clearing.
- Re-running `:parallel`/`:pa` while already enabled refreshes readiness and supervisor projection
  only; it does not reset the pool, reopen the automation epoch, or launch workers by itself.
- `Esc` closes the board surface only. Parallel mode remains enabled.
- `:parallel off`/`:pa off` disables local parallel mode and clears the automation epoch, pending
  dispatch, in-flight dispatch state, and any late post-turn parallel continuation result, but leaves
  pool worktrees in place.
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
- Unattended parallel workers use `workspace-write` and must leave source edits uncommitted. After
  `TurnCompleted`, Akra validates the exact Running lease, worktree root, branch, frozen base, and
  `HEAD`, then performs a bounded local host commit before it reserves official refresh order.
- The host commit path clears inherited Git control environment, disables fsmonitor, hooks, signing,
  and replacement objects, rejects active clean/process filters, and updates the source ref with an
  old-`HEAD` CAS. Assume-unchanged, skip-worktree, and unmerged index entries are preserved for
  operator recovery rather than cleared automatically. Top-level ignored build output stays outside
  the frozen source commit and is purged only by the post-integration, non-Running, identity-checked
  slot cleanup. Hidden dirty, untracked, ignored, or nested tracked-submodule state fails closed with
  submodule ignore settings overridden for inspection. Changed paths also reject symlinked or
  reparse-point ancestors, special files, hard-linked regular files, newly introduced Git gitlinks,
  files larger than 64 MiB, and aggregate changed content larger than 256 MiB. Path identity,
  link count, size, and modification metadata are compared immediately before and after staging.
  No-change/base-equivalent output, merge history, drift, ref races, or a dirty final worktree also
  fail closed before commit-ready. A clean existing linear descendant commit is accepted for
  compatibility. This pre/post comparison narrows deterministic staging attacks but is not an
  atomic filesystem transaction: another same-UID process can still race file contents between
  inspection and `git add`, so app-server process-tree termination and the final clean/status checks
  remain part of the containment contract.
- Agent completion becomes distributor-eligible only after hidden official planning refresh marks it
  commit-ready.
- A successful parallel official completion refresh that leaves another actionable queue head emits
  the next `parallel_official_completion` dispatch request, capped by idle-slot capacity.
- Distributor delivery is serial: source branch push, PR automation, integration into the configured
  integration branch (default `prerelease`), and slot cleanup.
- Slot acquisition freezes the push remote, credential-redacted canonical GitHub HTTPS URL,
  repository identity, visibility, integration branch, and fetched remote base OID before the
  worker starts. Enqueue carries that target with the source branch and source SHA. Legacy targets
  without the URL proof and any later URL/identity/visibility drift fail closed before remote
  writes. Frozen network Git and PR wrapper calls use a private isolated Git context, so mutable
  repository transport configuration and GitHub routing environment are not delivery inputs.
- Pool reset, reconcile, provisioning, cleanup, and lease creation require an isolated exact-target
  fetch proof before any slot mutation. A newly observed remote-only baseline advance blocks the
  current mutation pass and requires an explicit retry; stale remote-tracking refs are inspection
  data only. Invalid explicit integration-branch or push-remote configuration is unavailable and
  never falls back to `prerelease` or `origin` in background or destructive flows.
- PR mode defaults to `required`. Unless autonomous delivery is explicitly enabled, readiness
  requires `APPROVED`, `CLEAN`, passing checks, and a PR `headRefOid` equal to the frozen source SHA.
  `AKRA_GITHUB_PR_MODE=auto|disabled` or repo-local `akra.githubPrMode` never enables direct delivery
  alone; the parent process must start Akra with exact `AKRA_PARALLEL_AUTONOMOUS_DELIVERY=1`.
  Repository-local configuration cannot grant this high-risk permission.
- Human review/check gates are rechecked indefinitely at a bounded interval; network and remote
  command failures use persisted exponential backoff with an eight-attempt automatic retry limit.
- Public GitHub delivery requires the parent process to start Akra with exact
  `AKRA_PARALLEL_ALLOW_PUBLIC_REPOSITORY=1`. Repository-local configuration cannot grant this
  permission. Public without opt-in and unknown visibility block.
- `AKRA_PARALLEL_INTEGRATION_BRANCH=<branch>` or repo-local
  `git config akra.parallelIntegrationBranch <branch>` selects that branch, with the environment
  taking precedence. The branch must already exist on the configured remote; Akra does not seed it
  from the current workspace `HEAD`.
- `AKRA_GITHUB_PUSH_REMOTE=<remote>` or repo-local `git config akra.githubPushRemote <remote>`
  changes the remote used for source-branch publish, integration baseline fetch, GitHub repository
  discovery, interactive current-branch review polling, and integration-branch push when `origin`
  is not the correct delivery target. Invalid or missing explicit remotes fail closed rather than
  silently falling back to `origin`.
- Integration uses a generated detached worktree derived from the frozen target and never resets or
  moves the canonical checkout. Source-branch cleanup occurs only after remote integration
  verification and PR close, using a force-with-lease bound to the frozen source SHA; moved branches
  are preserved and surfaced in the durable delivery note.
- Queue admission freezes both the fetched integration merge-base and the reviewed source tip. The
  source range must be linear, contain 1 through 128 commits, and contain no merge commit. Delivery
  enumerates that exact oldest-to-newest range, requires GitHub's PR head to remain at the frozen
  tip, and cherry-picks every commit that is not already patch-equivalent in the target. Legacy or
  incomplete queue records are blocked before any remote side effect.

## Recovery Contract

- Store-backed claims coordinate official refresh and distributor queue-head processing.
- A distributor tick reconstructs a missing queue record for a still-running `commit_ready`
  session before processing the queue. The completion timestamp supplies deterministic ordering and
  identity, so a transient target fetch or process interruption cannot strand a reviewed result.
- Stale official refresh recovery may abandon only the current head order per pass.
- Retryable distributor recovery includes source-branch push failures, PR ensure/inspection failures,
  cherry-pick and integration-worktree precondition failures, GitHub automation or pull-request
  workflow unavailability, and cleanup-failure retries when the blocked record still matches the
  live lease/worktree.
- Integration delivery verifies the frozen remote source ref, complete source range, and PR head OID
  immediately before cherry-pick, then requires the pushed integration ref to equal the dedicated
  worktree HEAD.
  Integration push failures, branch divergence, dirty worktrees, and missing worktree evidence
  remain operator-owned; recovery never hard-resets local or canonical integration history.
- Failed-start dispatch blocks survive pool reset per task, keeping the latest `blocked_at`.
- Stale startup leases require matching session-detail evidence before automatic cleanup.
- Fresh same-epoch `Running` dispatch commands now recover immediately after restart/reentry when durable state shows no session detail yet, including the matching-lease/no-session handoff window.

## Current Limits

- Real-terminal validation remains required for restart, blocked distributor, and multi-worktree
  operator flows.
- Planning detail mode remains manual; `llm-assisted` authoring is disabled.
- Main-session command, file-change, and bounded permission approvals use a one-shot TUI modal.
  Supersession planning and parallel worker runtimes remain unattended and decline every approval.
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
