# Hexagonal Runtime Architecture

The stable dependency direction is still `adapter -> application -> domain`.

## Layer Ownership

- `src/adapter/inbound/tui`: operator input, reducer state, overlay state, and rendering
- `src/application/service`: use-case orchestration and outbound-port ownership
- `src/application/service/planning`: planning feature facades exposed to the TUI
- `src/application/port/outbound`: boundaries for app-server, filesystem, and worker execution
- `src/adapter/outbound`: concrete process and filesystem adapters
- `src/domain`: UI-neutral models and invariants

## Planning Boundary

- TUI code should depend on `PlanningFeature` only.
- `PlanningFeature` is split into `workspace`, `runtime`, and `worker` use cases.
- Planning internals such as validation, prompt assembly, reconciliation, and proposal promotion stay behind those facades.
- Planning-specific TUI flow lives under `src/adapter/inbound/tui/app/planning`.

## Invariants

- Mapping from protocol or filesystem shapes stays in adapters.
- Domain types stay free of TUI, transport, and filesystem concerns.
- New outbound capabilities still require ports owned by the application layer.
- If a TUI change needs planning internals directly, the planning facade is missing an operation and should be extended instead of bypassed.
