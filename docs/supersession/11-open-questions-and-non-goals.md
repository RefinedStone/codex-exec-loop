# Open Questions And Non-Goals

This document defines the target supersession model, not shipped behavior.

## V1 Non-Goals

- supervisor acting as a general implementation chat surface
- agents directly updating `task-ledger.json`
- more than one integration branch
- non-git workspaces
- distributed execution across multiple machines
- autonomous agent-to-agent conversation loops
- approval-review automation beyond currently available app-server capabilities

## Closed Decisions

The following are intentionally not open in v1:

- official task authority stays in `task-ledger.json`
- hidden planning worker remains planning-only
- execution unit is an agent session
- integration branch is `akra`
- pool default size is `3`
- distributor is serial
- agent completion contract is `commit ready`

## Open Questions

| Question | Why it remains open |
| --- | --- |
| exact stale-agent timeout policy | depends on real terminal behavior and app-server event cadence |
| branch-name truncation and collision policy | needs implementation constraints from git and remote naming |
| operator override flow for blocked merge queue items | depends on the final TUI interaction model |
| persistence depth for supervisor history | may be unnecessary for first shipped slice |
| whether blocked but pushed queue items may release slots early | depends on recovery and audit expectations |

## Later Extensions

- agent specialization by task category
- dynamic pool resizing without mode restart
- richer scheduling priority beyond queue order
- remote worker hosts
- review-thread summarization and richer GitHub awareness
- ledger-derived heuristics for grouping follow-up work by branch affinity

## Promotion Guidance

When parts of supersession ship:

- move shipped operator behavior into `docs/design`
- keep forward-looking or still-open mechanics in `docs/supersession`
- delete superseded planning notes once current contracts are documented elsewhere

## Related Docs

- [README.md](README.md)
- [01-product-model.md](01-product-model.md)
- [10-implementation-slices.md](10-implementation-slices.md)
- [../README.md](../README.md)

## Code Impact

Expected entrypoints:

- future impact spans `src/adapter/inbound/tui/app`
- future impact spans `src/application/service`
- future impact spans `src/adapter/outbound`
