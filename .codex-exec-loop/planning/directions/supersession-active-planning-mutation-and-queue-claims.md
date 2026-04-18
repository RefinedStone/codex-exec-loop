# Supersession active planning mutation and queue claims

## Outcome

Commit hidden planning refresh, queue projection rebuild, and supersession claim ownership through
one repo-scoped authority domain instead of separate file and runtime updates.

## Status

- Current branch status: implemented and recorded as `done` in `task-ledger.json`.
- Keep this file as compact rationale for what shipped; the remaining work moved on to validation, doc alignment, and residual polish rather than another queue-claim migration slice.

## Why this direction exists

The redesign only closes the current split-brain risk if active planning mutation, official refresh
reservation, and distributor queue-head claim live in one consistency domain. This is the slice
that turns the store from mirrored planning state into transactional supersession coordination.

## Long-horizon plan

- route hidden planner refresh through store-backed active commits
- rebuild queue projection in the same transaction as task-state change
- move official refresh claims and distributor queue-head claims into the authority domain
- keep `planning_revision` separate from `runtime_event_sequence`

## Near-term bias

- start with refresh commits and queue-head claim uniqueness
- prefer repeated queue-head regression coverage over event-breadth expansion
- make claim ownership and revision linkage explicit before recovery work grows

## Relevant inputs

- `docs/plan/18-repo-shared-planning-authority-store.md`
- `docs/plan/19-supersession-runtime-risk-audit.md`
- `docs/supersession/04-task-ledger-feedback-loop.md`
- `docs/supersession/09-architecture-boundaries.md`
- `docs/supersession/10-implementation-slices.md`

## Task derivation guidance

- derive one transaction boundary at a time: refresh commit, queue claim, revision update
- keep cross-process uniqueness and regression tests first-class
- tie new claims to explicit planning revision expectations

## Avoid

- separate stores for planning mutation and claim ownership
- incrementing planning revision for every runtime event
