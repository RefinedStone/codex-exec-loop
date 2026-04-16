# Supersession distributor merge queue

## Outcome

Move commit-ready agent results through one serial distributor that rebases stale branches, pushes,
opens or updates PRs, merges into `akra`, and returns cleaned slots to the pool.

## Why this direction exists

Parallel execution only helps if integration stays trustworthy. The supersession docs lock in one
single-consumer merge queue, one integration branch, and one explicit rebase strategy when `akra`
advances. That contract should drive implementation work instead of being inferred later.

## Long-horizon plan

- accept only `commit_ready` results into the merge queue
- process one queue item at a time from push through cleanup
- keep `akra` drift handling explicit through fetch, rebase, and `--force-with-lease` push
- surface degraded GitHub capability and blocked queue heads as operator-visible integration state

## Near-term bias

- land the local queue-state model and queue-head handling before full GitHub automation breadth
- keep rebase conflict handling explicit and operator-recoverable
- make slot return depend on cleanup success, not just merge success

## Relevant inputs

- `docs/supersession/05-git-worktree-pool.md`
- `docs/supersession/06-distributor-and-merge-queue.md`
- `docs/supersession/08-capabilities-degraded-mode-and-failures.md`
- `docs/supersession/10-implementation-slices.md`
- `docs/plan/04-worktree-branch-rules.md`
- `src/adapter/outbound`
- `src/application/port/outbound`

## Task derivation guidance

- derive slices around one queue-state or integration boundary at a time
- keep `blocked` and `failed` queue-head recovery visible before optimizing throughput
- prefer explicit provenance updates when rebase changes the source head

## Avoid

- parallelizing distributor work across multiple queue heads
- leaving `akra` drift or PR-state handling implicit in the contract
