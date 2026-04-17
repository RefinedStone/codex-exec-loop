# Supersession architecture boundaries

## Outcome

Split supersession into explicit ports, services, and runtime seams so parallel mode can ship
without bloating the current single-session shell and planning hotspots.

## Why this direction exists

The supersession docs call for a new orchestration layer, not a thin extension of the current
conversation runtime. That means new boundaries for agent sessions, git worktrees, distributor, and
GitHub automation, while keeping planning authority behind the existing planning facade.

## Long-horizon plan

- add dedicated domain snapshots for supervisor, slots, agents, capabilities, and merge queue
- add explicit outbound ports for agent session, git workspace, distributor, and GitHub automation
- separate normal-mode runtime from parallel-mode runtime
- shrink current hotspots by extracting supersession-specific logic into focused submodules

## Near-term bias

- extract only the seams needed by the first five supersession directions
- reuse hidden planning worker and planning runtime services through explicit facade seams
- keep refactors tied to visible supersession behavior, not abstract cleanup

## Relevant inputs

- `docs/supersession/09-architecture-boundaries.md`
- `docs/supersession/10-implementation-slices.md`
- `docs/design/04-hexagonal-runtime-architecture.md`
- `src/adapter/inbound/tui/app/app_runtime.rs`
- `src/adapter/inbound/tui/app/conversation_runtime.rs`
- `src/adapter/inbound/tui/app/shell_presentation.rs`
- `src/application/service/session_service.rs`
- `src/adapter/outbound/codex_app_server_adapter.rs`

## Task derivation guidance

- every refactor slice should name the supersession behavior it unlocks
- prefer new submodules and projections over widening existing mega-files
- keep planning authority and supersession execution concerns separated in naming and tests

## Avoid

- broad runtime rewrites that do not unlock one active supersession slice
- folding supersession state back into the current single conversation model
