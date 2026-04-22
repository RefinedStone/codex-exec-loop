# Remaining Supersession Work

This file tracks only the supersession, planning, and directions work that is still unfinished,
lightly validated, or intentionally deferred.

Implemented behavior is summarized in [current-contract.md](current-contract.md).

## Validation And Operations

- run real-terminal checks for restart recovery, blocked distributor flows, and multi-worktree
  operator behavior
- keep validation artifacts focused on the current authority-store path, not just the original
  `prerelease` loop
- add directions-focused TUI end-to-end coverage for save/promote, detail-doc repair, queue-idle
  prompt editing, and overview refresh after promote
- keep failure cases grounded in observed shell, git, and GitHub behavior rather than purely
  unit-level assumptions

## Docs And Copy Polish

- keep `docs/supersession/` as the canonical current-contract hub as new slices land
- compact implemented areas aggressively instead of reopening broad historical summaries
- tighten supersession pause reasons, recovery guidance, queue alerts, and completion wording so
  they read as operator actions rather than internal state
- keep planning directions and queue-idle wording aligned with the reduced docs structure

## Open Questions

| Question | Why it remains open |
| --- | --- |
| exact stale-agent timeout policy | depends on real terminal behavior and app-server event cadence |
| operator override flow for blocked merge queue items | depends on the final TUI interaction model |
| persistence depth for supervisor history | may be unnecessary for the first shipped slice |
| whether blocked but pushed queue items may release slots early | depends on recovery and audit expectations |

## Explicit Non-Goals

- supervisor acting as a general implementation chat surface
- agents directly updating active planning authority
- more than one integration branch
- non-git workspaces
- distributed execution across multiple machines
- autonomous agent-to-agent conversation loops
- approval-review automation beyond currently available app-server capabilities

## Later Extensions

- agent specialization by task category
- dynamic pool resizing without mode restart
- richer scheduling priority beyond queue order
- remote worker hosts
- review-thread summarization and richer GitHub awareness
- ledger-derived heuristics for grouping follow-up work by branch affinity
