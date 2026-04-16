# Architecture boundaries for UX iteration

## Outcome

Keep UX and runtime improvements shippable by reducing structural hotspots only when they unlock safer operator-facing work.

## Why this direction exists

Large shell and planning hotspots make status-language work, queue presentation work, and planning authoring work harder to ship coherently. The target is not elegance for its own sake; it is safer iteration.

## Long-horizon plan

- separate status wording and projection from layout and rendering
- separate conversation lifecycle from automation lifecycle
- separate planning authoring flow from planning runtime flow
- organize tests around operator journeys instead of only current file boundaries

## Near-term bias

- extract presentation seams that unblock Phase 1 operator copy work
- create smaller reviewable slices around shell status, queue presentation, or planning authoring
- use the architecture debt map as the refactor source of truth

## Relevant inputs

- `docs/plan/17-structure-and-architecture-debt-map.md`
- `src/adapter/inbound/tui/app/shell_presentation.rs`
- `src/adapter/inbound/tui/app/conversation_runtime.rs`
- `src/adapter/inbound/tui/app/planning/controller.rs`
- `src/application/service/planning_runtime_facade_service.rs`

## Task derivation guidance

- only derive refactor tasks that unlock a specific operator-visible improvement
- make the user-facing benefit explicit in `direction_relation_note`
- keep test changes aligned with the extracted boundary

## Avoid

- broad cleanup tasks with no clear product outcome
- refactors that touch many hotspots without reducing conceptual coupling
