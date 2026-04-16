# Agent Session Lifecycle

This document defines the target supersession model, not shipped behavior.

## Execution Contract

An agent session is a main-grade codex/app-server session with:

- one assigned task
- one leased slot
- one dedicated worktree
- one active branch
- one running-duration clock

It is not a hidden worker and it is not a detached planning helper.

## Lifecycle States

| State | Meaning | Enters from | Exits to |
| --- | --- | --- | --- |
| `requested` | supervisor chose a ledger-backed task for execution | supervisor assignment | `assigned`, `failed` |
| `assigned` | slot and branch reserved, session launch pending | `requested` | `starting`, `failed` |
| `starting` | app-server thread/session bootstrap in progress | `assigned` | `running`, `failed` |
| `running` | agent actively executes in its worktree | `starting`, `resumed` | `reported_complete`, `failed`, `cancelled` |
| `reported_complete` | agent claims task milestone reached | `running` | `ledger_refreshing`, `failed` |
| `ledger_refreshing` | hidden planning worker is refreshing official task state | `reported_complete` | `commit_ready`, `failed` |
| `commit_ready` | agent result and ledger state now agree that execution milestone finished | `ledger_refreshing` | `merge_queued`, `failed` |
| `merge_queued` | distributor accepted the result for integration | `commit_ready` | `merged`, `failed` |
| `merged` | distributor integrated the result into `akra` | `merge_queued` | `cleanup_pending`, `failed` |
| `cleanup_pending` | agent execution is over but the leased slot still needs reconcile or release | `merged`, `failed`, `cancelled` | `cleaned`, `failed` |
| `cleaned` | slot returned to clean `akra` state and lease released | `cleanup_pending` | terminal |
| `failed` | session, planning, git, or integration step needs operator action | any | `cleanup_pending`, `requested`, terminal |
| `cancelled` | operator explicitly stopped the session before commit-ready completion | `running`, `assigned` | `cleanup_pending`, terminal |

## Assignment Preconditions

Supervisor may create an agent only when all of the following are true:

- parallel mode is enabled
- planning is not invalid
- the ledger contains at least one executable ready task
- the pool has at least one idle slot
- distributor is not holding a global blocking failure that forbids new work

If any precondition fails, no session is created and the supersession surface must explain why.

## Running Duration

`running duration` starts when the agent enters `running` and stops when it enters one of:

- `reported_complete`
- `failed`
- `cancelled`

If the agent later resumes after operator intervention, resume creates a new running segment while
keeping cumulative active duration in history.

## Completion Contract

V1 completion is `commit ready`. The agent must:

- finish local edits for the assigned task
- run or report the required validation commands for that task
- produce a local commit on its assigned branch
- report a structured completion summary back to supervisor

The agent does not:

- update `task-ledger.json` directly
- merge into `akra`
- decide official next tasks
- declare global completion without ledger refresh

## Reported Versus Official Completion

`reported_complete` means the agent claims it is done.
`commit_ready` means hidden planning worker has refreshed the ledger and supervisor now accepts the
result as an official milestone.

This distinction is mandatory because the ledger remains the official source of task state.

## Interruption And Failure Rules

### Cancel

- operator may cancel an agent before `commit_ready`
- cancelled agents keep their slot lease until supervisor moves them into `cleanup_pending`
- `cleanup_pending` may archive, inspect, or clean the worktree before the slot becomes reusable

### Hung Session

- if no event, tool activity, or summary update appears for the configured timeout window, mark the agent `failed`
- a failed hung session does not auto-release the slot and must later enter `cleanup_pending`

### Planning Refresh Failure

- if hidden planning worker cannot refresh the ledger, the agent stops at `failed`
- the slot stays reserved because the completion result is not yet official
- operator recovery may either retry planning or move the slot into `cleanup_pending` to discard execution state

### Distributor Failure

- once an agent reaches `merge_queued`, execution is finished but integration may still fail
- the session itself does not resume; the queue item carries the failure forward

### Cleanup Pending

- `cleanup_pending` bridges agent-terminal states and slot-terminal reuse
- merged agents always pass through `cleanup_pending` before `cleaned`
- failed or cancelled agents also pass through `cleanup_pending` when the operator chooses discard, archive, or reset
- a slot is never reusable directly from `failed` or `cancelled`

## Reassignment Rules

- a slot may be reassigned only after `cleaned`
- a task may be reassigned only after its previous agent ends in `failed` or `cancelled`
- reassignment must reference the latest ledger state, not the stale pre-failure assignment snapshot

## Related Docs

- [01-product-model.md](01-product-model.md)
- [04-task-ledger-feedback-loop.md](04-task-ledger-feedback-loop.md)
- [05-git-worktree-pool.md](05-git-worktree-pool.md)
- [06-distributor-and-merge-queue.md](06-distributor-and-merge-queue.md)

## Code Impact

Expected entrypoints:

- `src/adapter/outbound/codex_app_server_adapter.rs`
- `src/application/service/session_service.rs`
- `src/adapter/inbound/tui/app/conversation_runtime.rs`
