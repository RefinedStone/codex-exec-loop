# Supersession git worktree pool

## Outcome

Manage `akra pool` as a collision-resistant set of git worktree slots that can be leased to agent
sessions and returned only after clean `akra` reconcile.

## Why this direction exists

The docs define pool capacity as the hard bound on parallel execution. That means slot identity,
hash-safe pool roots, lease states, exhaust handling, and cleanup rules are product behavior rather
than implementation detail.

## Long-horizon plan

- reconcile a default pool of three slots rooted outside the integration checkout
- keep slot state explicit as idle, leased, running, awaiting cleanup, blocked, or missing
- enforce clean `akra` state before any slot becomes reusable
- surface pool exhaustion and blocked-slot recovery as operator-facing state

## Near-term bias

- land slot reconcile and cleanliness checks before broader scaling behavior
- keep default pool root collision-resistant for sibling clones
- make `cleanup_pending` and blocked-slot recovery first-class

## Relevant inputs

- `docs/supersession/05-git-worktree-pool.md`
- `docs/supersession/08-capabilities-degraded-mode-and-failures.md`
- `docs/supersession/10-implementation-slices.md`
- `docs/plan/04-worktree-branch-rules.md`
- `src/adapter/inbound/cli.rs`
- `src/application/port/outbound`
- `src/adapter/outbound`

## Task derivation guidance

- derive slices around one pool lifecycle concern at a time
- keep slot state and lease bookkeeping operator-readable in every slice
- prefer deterministic cleanup rules over implicit reuse

## Avoid

- reusing dirty slots because capacity is low
- treating worktree root naming or cleanup verification as optional polish
