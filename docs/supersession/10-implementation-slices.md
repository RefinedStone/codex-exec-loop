# Remaining Supersession Work

This file tracks only the supersession work that is still unfinished, lightly validated, or worth
keeping as an explicit follow-through item.

Implemented supersession contracts have been merged into
[implemented-summary.md](implemented-summary.md).

## Already Complete

- `origin/prerelease` ships readiness gating, the supervisor board, the worktree pool, official completion staging, and serial distributor delivery.
- The current branch also ships the repo-scoped authority locator, store-backed drafts and promote, active planning mutation and claims, runtime projection recovery, store-primary reads, and legacy bootstrap cleanup.

## Remaining Detailed Work

### Validation Depth

- run real-terminal checks for restart recovery, blocked distributor flows, and multi-worktree operator behavior
- capture validation artifacts for the current authority-store path, not just the original prerelease loop
- keep failure cases grounded in observed shell, git, and GitHub behavior rather than purely unit-level assumptions

### Docs Alignment

- keep `docs/design/` as the only detailed current-truth lane
- keep `docs/supersession/` focused on remaining work and merged history instead of repeating shipped contracts
- keep planning directions and queue-idle prompts aligned with the reduced docs structure

### Residual Surface Polish

- tighten supersession copy where pause reasons or recovery guidance still read like internal state
- keep queue, alert, and completion wording operator-actionable
- avoid reopening architecture slices unless validation reveals a concrete regression

## Acceptance

- current docs no longer duplicate the same supersession contract across `design`, `supersession`, and planning directions
- remaining supersession docs describe only unresolved validation, polish, or open-question work
- terminal validation coverage exists for the remaining recovery-sensitive paths
