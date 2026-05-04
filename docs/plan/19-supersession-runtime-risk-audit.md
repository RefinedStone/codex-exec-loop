# Supersession Runtime Risk Audit

Historical reference only. Use
[../supersession/current-contract.md](../supersession/current-contract.md) for the current
supersession contract and [../supersession/remaining-work.md](../supersession/remaining-work.md)
for unfinished work.

## Status

The original long audit described pre-store-primary supersession risks. The current branch has
closed the main structural risks through DB-backed planning authority, store-backed official refresh
claims, distributor queue claims, runtime projections, guarded pool reset, and targeted recovery.

## Closed Risk Set

| ID | Historical risk | Current resolution |
| --- | --- | --- |
| R1 | root checkout and leased worktrees could observe different planning authority | accepted planning authority is repo-scoped and DB-backed |
| R2 | task ledger and queue projection could diverge across file writes | accepted task authority and queue projection commit through one store path |
| R3 | official completion ordering was process-local | official refresh order and claims are store-backed |
| R4 | distributor queue head lacked durable claim semantics | queue item claims and runtime queue records are stored |
| R5 | slot leases and session detail could lose updates through file-only mirrors | runtime projections are stored and mirrored as compatibility artifacts |
| R6 | planning authority could leak into agent worktree branches | tracked planning files are review/export/staged-edit artifacts only |
| R7 | restart could forget refresh or delivery state | recovery sweeps recheck store, git, claims, queue records, and session detail |
| R8 | exported queue views could drift from runtime truth | runtime reads accepted DB authority and derived projections |

## Remaining Risk References

- Real-terminal restart and blocked-queue validation: [../supersession/remaining-work.md](../supersession/remaining-work.md)
- Architecture/debt ordering: [17-structure-and-architecture-debt-map.md](17-structure-and-architecture-debt-map.md)
- Authority-store design history: [18-repo-shared-planning-authority-store.md](18-repo-shared-planning-authority-store.md)
