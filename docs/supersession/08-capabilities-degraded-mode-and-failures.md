# Capabilities, Degraded Mode, And Failures

This document defines the target supersession model, not shipped behavior.

## Capability Philosophy

Parallel mode should fail by surfacing readiness, not by crashing the application.
If a prerequisite is missing, supersession should enter a degraded or blocked state that clearly
names the missing capability and the next operator action.

## Capability Check Matrix

| Check | Why it matters | Failure effect |
| --- | --- | --- |
| git repository present | supersession requires branch and worktree control | parallel mode cannot activate |
| `git worktree` available | slots are worktree-backed | pool cannot reconcile |
| `akra` branch available or creatable | integration target must exist | supersession blocked before assignment |
| push capability available | distributor cannot publish results otherwise | merge queue blocked |
| `gh` binary installed | end-to-end GitHub automation depends on it | GitHub flow degraded |
| `gh auth status` succeeds | PR automation needs valid auth | GitHub flow degraded |
| planning workspace valid | ledger authority must be trustworthy | assignment blocked |

## Readiness States

| State | Meaning |
| --- | --- |
| `ready` | all capabilities required for full supersession flow are available |
| `degraded` | supersession can run, but one or more downstream steps need manual recovery |
| `blocked` | supersession cannot safely assign or integrate work |
| `repairing` | the system is actively trying to restore capability or planning state |

## Degraded Mode Rules

Supersession may still open in degraded mode when:

- git exists but push is unavailable
- git exists but `gh` is unavailable or unauthenticated
- some slots are blocked but at least one idle slot remains

Degraded mode should:

- allow inspection
- allow already-safe local orchestration when possible
- refuse actions that require the missing capability
- keep blocked steps visible until resolved

## Failure Taxonomy

| Failure | Current state | Next action | Auto-retry | Scope |
| --- | --- | --- | --- | --- |
| not a git repo | blocked | leave parallel mode or open a git-backed workspace | no | whole mode |
| `akra` create failed | blocked | inspect branch permissions and repo state | bounded | whole mode |
| pool reconcile failed | degraded or blocked | retry reconcile and inspect blocked slots | bounded | pool |
| no idle slot | running or degraded | wait, recover, or increase pool size later | no | assignment |
| planning invalid | blocked | repair planning and rerun ledger checks | bounded through planning flow | assignment |
| ledger refresh failed | failed | inspect completion payload and repair planning | bounded | one agent plus assignment queue |
| push failed | blocked | restore remote access and retry distributor | bounded | merge queue |
| `gh` auth failed | degraded | restore auth and retry distributor | bounded | merge queue |
| merge conflict | blocked | resolve conflict or discard queue item | no | queue head |
| cleanup verify failed | blocked | inspect slot worktree and clean manually | no | one slot |

## Operator Messaging Contract

Each surfaced issue should state:

- current state
- cause
- next action

Example pattern:

- `blocked: merge queue paused`
- `cause: slot-2 branch could not merge into akra cleanly`
- `next action: inspect the queue head and resolve the conflict before retrying distributor`

## Planning Readiness Boundary

Because the ledger is the official task source of truth:

- invalid planning blocks new assignment
- invalid planning does not erase already-visible supersession history
- planning recovery must point back to the existing planning authoring surfaces

## Related Docs

- [04-task-ledger-feedback-loop.md](04-task-ledger-feedback-loop.md)
- [05-git-worktree-pool.md](05-git-worktree-pool.md)
- [06-distributor-and-merge-queue.md](06-distributor-and-merge-queue.md)
- [07-supervisor-ui-and-surfaces.md](07-supervisor-ui-and-surfaces.md)

## Code Impact

Expected entrypoints:

- `src/adapter/inbound/tui/app/shell_presentation.rs`
- `src/application/service/planning_doctor_service.rs`
- `src/adapter/inbound/cli.rs`
