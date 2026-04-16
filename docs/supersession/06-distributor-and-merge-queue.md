# Distributor And Merge Queue

This document defines the target supersession model, not shipped behavior.

## Distributor Role

The distributor is the only subsystem allowed to integrate agent results into the `akra` branch.
It is deliberately separate from agent execution so that:

- agent sessions stay focused on task completion
- integration stays serialized
- cleanup and slot return have one owner

## Queue Ownership

Only `commit_ready` agent results may enter the merge queue.

Each queue item must contain:

- agent id
- task id
- source branch
- source worktree path
- commit sha
- validation summary
- ledger refresh version or timestamp
- GitHub capability snapshot taken when enqueued

## Queue Item States

| State | Meaning |
| --- | --- |
| `queued` | waiting for distributor turn |
| `pushing` | branch push in progress |
| `pr_pending` | branch pushed, PR ensure/create in progress |
| `merge_pending` | PR exists and merge prerequisites are being checked |
| `integrating` | local and remote integration into `akra` is happening |
| `cleaning` | source slot is being reset back to clean `akra` |
| `done` | integration and cleanup succeeded |
| `blocked` | cannot continue without operator action |
| `failed` | terminal error for this queue item |

## Serial Processing Rule

The distributor is a single-consumer system.

Closed v1 rules:

- only one queue item may be processed at a time
- no agent result may bypass the queue
- queue order follows commit-ready acceptance order
- blocked queue head prevents later items from integrating until resolved or removed

## End-To-End Flow

For one queue item, distributor runs:

1. verify source worktree is still available
2. verify commit sha still matches expected branch head
3. verify source worktree is not mid-merge or otherwise corrupted
4. push source branch
5. ensure PR exists through `gh`
6. evaluate merge readiness
7. update local `akra`
8. integrate the source result into `akra`
9. push `akra`
10. close or mark PR merged if applicable
11. clean the source slot back to `akra`
12. mark queue item `done`

## GitHub Capability Contract

| Capability state | Distributor behavior |
| --- | --- |
| `gh` installed and authenticated | create or inspect PR, merge, and close through `gh` |
| push available but `gh` unavailable | push may proceed, but queue item becomes `blocked` before PR/merge stage |
| push unavailable | queue item becomes `blocked` immediately |

The system reports these states as readiness, not app crashes.

## Merge Policy

V1 merge policy is linear integration into `akra`.

- no GitHub merge commits
- distributor is responsible for keeping `akra` current before integrating
- if merge conflict occurs, mark the queue item `blocked`
- blocked merge conflict keeps the slot unavailable until operator resolves or discards the item

## Akra Drift And Rebase Strategy

If `akra` advanced after the agent became `commit_ready`, distributor must reconcile the source
branch before integration.

Required strategy:

1. fetch latest remote refs
2. refresh local `akra` to the expected integration head
3. compare the source branch base against current `akra`
4. if `akra` moved, rebase the source branch onto latest `akra`
5. if rebase succeeds, update the queue item's head commit reference
6. push the rebased branch with `--force-with-lease`
7. continue PR and merge checks against the rebased head only

If rebase conflicts:

- mark the queue item `blocked`
- preserve both pre-rebase and attempted-head provenance in the queue item detail
- do not advance to local integration or slot cleanup

This keeps the policy linear while removing ambiguity about how stale agent branches re-enter the
integration path.

## Cleanup And Slot Return

Distributor owns slot return after integration. A queue item is not `done` until:

- `akra` contains the result
- source slot checks out `akra`
- worktree is clean
- slot lease is released

This keeps "merged" separate from "reusable".

## Retry Rules

- transient push or `gh` command failures may auto-retry with bounded attempts
- merge conflicts never auto-retry without new operator action
- cleanup verification failures never silently downgrade to success
- blocked queue items preserve diagnostic context for later retry

## Related Docs

- [05-git-worktree-pool.md](05-git-worktree-pool.md)
- [08-capabilities-degraded-mode-and-failures.md](08-capabilities-degraded-mode-and-failures.md)
- [10-implementation-slices.md](10-implementation-slices.md)
- [../plan/04-worktree-branch-rules.md](../plan/04-worktree-branch-rules.md)

## Code Impact

Expected entrypoints:

- `src/application/port/outbound`
- `src/adapter/outbound`
- `src/adapter/inbound/tui/app`
