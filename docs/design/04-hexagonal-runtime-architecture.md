# Runtime Boundary Architecture

This is the stable architecture reference for the native-first client.

The short rule is:

```text
adapter/inbound/* -> core or application -> domain
core -> application -> domain
application -> outbound ports -> adapter/outbound/*
composition -> concrete wiring
```

`src/core` is a headless app runtime boundary. It is not a replacement for the
domain model, application services, or outbound adapters. Its job is to keep the
interactive app flow deterministic across TUI, app-server, and future inbound
surfaces.

## Layer Ownership

| Layer | Owns | Must not own |
| --- | --- | --- |
| `adapter/inbound/*` | Input parsing, rendering, local presentation state, route or bot command mapping | Domain policy, dispatch policy, durable task truth |
| `core` | App-level command/event/effect flow, runtime state, background completions, projections, snapshot production | Ratatui/Crossterm types, HTTP route types, Telegram types, concrete DB/git/filesystem adapters |
| `application/service` | Use-case orchestration, ordering gates, service transactions, port calls, planning and parallel control-plane handles | UI widgets, terminal events, transport-specific request types |
| `application/port` | Outbound contracts required by application services | Concrete adapter implementation details |
| `domain` | Pure decisions, invariants, validation, state transitions | Runtime, async, filesystem, database, git, UI, logging side effects |
| `adapter/outbound/*` | Concrete integrations for app-server, DB, filesystem, git, GitHub, Telegram | Application policy or domain invariants |
| Composition | Dependency construction and concrete wiring | Business decisions |

Mapping logic stays in adapters. Policy stays in domain or application services.
`core` coordinates when app-level work happens and how results are projected
back to inbound adapters.

## Core Runtime Boundary

`src/core` should speak in explicit app contracts:

- `AppCommand`: user or adapter intent.
- `CoreInput`: a command, background completion, tick, or lifecycle input.
- `Effect`: work requested by core but executed outside core.
- `Completion`: side-effect result that re-enters core through the same input
  queue.
- `AppEvent`: externally useful app transition.
- `AppSnapshot` or projection: read model for adapters.

The TUI may convert key events into `AppCommand` values and render
`AppSnapshot` values. It should not duplicate lifecycle orchestration that core
already owns.

Current migrated responsibilities include startup/session loading, conversation
selection, turn submission, stream reduction, manual prompt submission, and
post-turn evaluation. Parallel-mode mutation still goes through the application
control-plane handle described in
[`05-parallel-control-plane-architecture.md`](./05-parallel-control-plane-architecture.md).

`tests/architecture_boundaries.rs` keeps the core/application ownership gates
active. Treat any new exception or debt entry there as a regression unless the
tradeoff is explicitly documented with a removal path.

## State Ownership

| State kind | Owner |
| --- | --- |
| Cursor, modal, overlay, local editor buffer, selected row | Inbound adapter |
| Session/conversation app lifecycle, in-flight app effects, stream reduction state | `core` |
| Parallel wake coalescing, effect ordering, runtime loop state | Application control-plane |
| Task authority, dispatch commands, leases, session records, distributor queue | Durable repository/store |
| Eligibility, capacity, retry, stale-event, and validation decisions | Domain aggregate or domain service |

If state affects a domain invariant or must survive a restart, it does not
belong in TUI state. If state only affects rendering or focus, it should not be
promoted into domain or application services.

## Planning Boundary

Planning data follows the same rule:

```text
adapter/inbound/tui -> application/service/planning -> application/port -> adapter/outbound/{db,filesystem}
```

The durable planning authority is the SQLite-backed store. Filesystem plan
artifacts are projections/workspace files. The TUI can hold temporary form state
and selected IDs, but it must not decide durable planning truth.

## Forbidden Directions

- `domain` importing `application`, `core`, `adapter`, async runtime, or IO
  crates.
- `application` importing `core`, TUI, HTTP, Telegram, Ratatui, Crossterm, or
  concrete outbound adapter modules.
- `core` importing inbound adapter UI/transport types or concrete outbound
  adapters.
- TUI calling raw planning or parallel services when a core command or
  application control-plane handle exists.
- Outbound adapters encoding policy that belongs in application or domain.

## Small-Context Rules

- Keep DTOs and mappings close to the boundary that needs them.
- Add a port only when a real outbound boundary exists.
- Prefer small request/response structs over generic maps.
- Keep command/effect/completion names explicit and boring.
- Write architecture tests when a boundary is easy to regress.

## Verification

Use these gates for boundary-sensitive work:

```bash
source "$HOME/.cargo/env"
cargo test --test architecture_boundaries
cargo test
cargo fmt --check
```

For broad native/TUI changes also run:

```bash
bash scripts/check_native_pr.sh
```
