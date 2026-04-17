# Supersession runtime projections and recovery

## Outcome

Move slot, session, distributor, and recovery projections into the repo-shared authority domain and
recover in-flight work by reconciling committed state with Git and GitHub truth.

## Why this direction exists

Store-backed claims are not enough on their own. Supersession still needs durable runtime
projections plus restart recovery that re-checks external truth before deciding whether in-flight
refresh, push, PR, integration, or cleanup work already finished, failed, or needs operator help.

## Long-horizon plan

- store slot, session, queue, and distributor delivery projections in the authority domain
- append runtime-domain events with observed planning revision
- detect orphaned or timed-out claims
- reconcile Git, GitHub, and worktree truth before reclassifying in-flight work

## Near-term bias

- start with durable projections and recovery state classification
- treat external truth rechecks as mandatory for delivery-side recovery
- keep recovery states explicit and operator-readable

## Relevant inputs

- `docs/plan/18-repo-shared-planning-authority-store.md`
- `docs/plan/19-supersession-runtime-risk-audit.md`
- `docs/supersession/06-distributor-and-merge-queue.md`
- `docs/supersession/09-architecture-boundaries.md`
- `docs/supersession/10-implementation-slices.md`

## Task derivation guidance

- derive one recovery boundary at a time: projections, claims, external recheck, reclassification
- keep restart scenarios and orphaned-claim cases explicit in tests
- validate recovery behavior against real Git and GitHub truth, not projection replay alone

## Avoid

- replay-only recovery that skips external truth rechecks
- in-flight state transitions that cannot explain operator-visible recovery status
