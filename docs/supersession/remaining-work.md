# Remaining Supersession Work

Only track unfinished, lightly validated, or intentionally deferred work here. Implemented behavior
belongs in [current-contract.md](current-contract.md).

## Validation

- Run real-terminal checks for restart recovery, blocked distributor flows, and multi-worktree
  operator behavior.
- Keep validation artifacts focused on the DB authority-store path.
- Add directions-focused TUI end-to-end coverage for save/promote, detail-doc repair,
  queue-idle prompt editing, and overview refresh after promote.
- Keep failure cases grounded in observed shell, git, and GitHub behavior.

## Copy And Operations

- Tighten supersession pause reasons, recovery guidance, queue alerts, and completion wording so
  they read as operator actions.
- Keep planning directions and queue-idle wording aligned with the compact docs structure.

## Open Questions

| Question | Why it remains open |
| --- | --- |
| stale-agent timeout policy | needs real terminal and app-server cadence evidence |
| operator override flow for blocked merge queue items | depends on final TUI recovery interaction |
| persistence depth for supervisor history | may be unnecessary for the shipped slice |
| whether blocked but pushed queue items may release slots early | depends on recovery and audit expectations |

## Non-Goals

- supervisor as general implementation chat
- agents directly mutating active planning authority
- more than one integration branch
- non-git supersession pool semantics
- distributed execution across machines
- autonomous agent-to-agent conversation loops

## Later Extensions

- agent specialization by task category
- dynamic pool resizing without mode restart
- richer scheduling priority beyond queue order
- remote worker hosts
- richer GitHub review-thread awareness
