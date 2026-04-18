# Open Questions And Non-Goals

Keep this file limited to genuinely open supersession questions and explicit non-goals.

## V1 Non-Goals

- supervisor acting as a general implementation chat surface
- agents directly updating active planning authority
- more than one integration branch
- non-git workspaces
- distributed execution across multiple machines
- autonomous agent-to-agent conversation loops
- approval-review automation beyond currently available app-server capabilities

## Closed Decisions

- official task authority lives in one repo-scoped planning authority domain
- tracked planning files are review or portability artifacts, not runtime authority
- `draft -> validate -> promote` remains the operator authoring contract
- hidden planning worker remains planning-only
- execution unit is an agent session
- integration branch is `akra`
- pool default size is `3`
- distributor is serial
- agent completion contract is `commit ready`
- agent branch names use sanitized task slugs, deterministic hash truncation, and numbered collision suffixes

## Open Questions

| Question | Why it remains open |
| --- | --- |
| exact stale-agent timeout policy | depends on real terminal behavior and app-server event cadence |
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

Related remaining-work context lives in [10-implementation-slices.md](10-implementation-slices.md).
