# Parallel Control-Plane Architecture

This document is the canonical architecture reference for parallel mode,
supersession, and task dispatch.

The short rule is:

```text
TUI intent
  -> application control-plane handle
  -> core runtime projection bridge
  -> domain aggregate decision
  -> durable store / effect runner
  -> projection
  -> TUI rendering
```

The TUI sends intent. The application control-plane serializes mutation and
effect ordering, and the core runtime projection bridge keeps the headless app
runtime aligned with the control-plane read model. The domain decides policy.
The repository/store is the durable source of truth.

## R6 Runtime Decision

The current implementation uses a mutex-serialized synchronous facade around
the parallel control-plane runtime. It is intentionally not a mailbox actor loop.

This keeps the control-plane small while still providing:

- single-writer mutation through `ParallelModeControlPlaneHandle`;
- in-flight effect accounting;
- stale completion dropping;
- wake coalescing;
- durable dispatch-command backpressure;
- one projection source for inbound adapters.

Do not add a second runtime, queue actor, or direct raw `ParallelModeService`
owner inside `core` or TUI unless this decision is explicitly revisited.

## Layer Ownership

| Layer | Allowed | Forbidden |
| --- | --- | --- |
| TUI | Toggle/request/pause/resume intent, selection, overlays, loading markers, rendering snapshots | Capacity math, worker launch decisions, retry decisions, dispatch ordering |
| `core` | Copy application projections into app snapshots, route user intent to the application handle | Owning raw parallel services, creating a second queue, deciding parallel policy |
| Application control-plane | Serialize command handling, coordinate effects, maintain runtime state, expose projections | UI rendering, terminal key handling, durable policy hidden outside domain |
| Domain | Decide eligibility, capacity, stale-event behavior, dispatch state transitions, validation | IO, async runtime, UI, filesystem, database calls |
| Repository/store | Persist authority state, leases, dispatch commands, session records, queue state | Business rules that should be tested in domain |
| Effect runner | Execute concrete side effects requested by the control-plane | Deciding whether the side effect should exist |

## State Ownership

| State | Owner |
| --- | --- |
| Board selection, overlay visibility, prompt lock display | TUI |
| Latest parallel snapshot/status shown to the user | TUI projection cache |
| Runtime wake scheduling, effect IDs, stale completion guards | Application control-plane |
| Task authority, dispatch queue, leases, session records | Durable store |
| Capacity, readiness, worker actionability, supersession validity | Domain |

When in doubt, ask whether the state must survive process restart or affects a
domain invariant. If either is true, it should not live only in TUI state.

## Command Flow

1. The TUI turns a key binding or command into an application intent.
2. The control-plane handle serializes the command.
3. The control-plane loads the necessary authority state and asks domain code
   for the decision.
4. The control-plane records the state transition and emits effects.
5. The effect runner executes side effects and returns completions.
6. Completions re-enter the control-plane and stale completions are discarded.
7. The TUI renders the latest projection without recalculating policy.

## Core Integration

`core` may include parallel projections in `AppSnapshot` so inbound adapters can
render a single app view. That does not make core the owner of parallel
mutation. Parallel commands continue to go through
`ParallelModeControlPlaneHandle`, and the domain/application layers continue to
own policy and ordering.

## Boundary Gates

`tests/architecture_boundaries.rs` enforces the current boundary: core must not
depend on application DTOs, runtime workers, raw services, or parallel
control-plane internals. New work should keep those gates green. Any proposed
exception is architecture debt and needs an explicit removal path before it is
accepted.

## Review Checklist

- Does the TUI only send intent and render projection?
- Is mutation serialized through `ParallelModeControlPlaneHandle`?
- Are eligibility, capacity, retry, and stale-event decisions in domain code?
- Is durable truth written through the repository/store boundary?
- Are side effects represented as effects and completed back into the
  control-plane?
- Did architecture-boundary tests stay green or move debt downward?

## References

- [`04-hexagonal-runtime-architecture.md`](./04-hexagonal-runtime-architecture.md)
- [`08-parallel-mode-supersession-board.md`](./08-parallel-mode-supersession-board.md)
- [`../supersession/current-contract.md`](../supersession/current-contract.md)
- [`../../tests/architecture_boundaries.rs`](../../tests/architecture_boundaries.rs)
