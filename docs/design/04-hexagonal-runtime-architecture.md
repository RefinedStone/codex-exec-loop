# Hexagonal Runtime Architecture

The stable dependency direction is still `adapter -> application -> domain`.

This repo also treats small-context readability as a design requirement. The target is not just clean
ownership, but a layout where one operator-visible flow can usually be understood without opening
every adapter and every helper file in the repo.

## Layer Ownership

- `src/adapter/inbound/tui`: operator input, reducer state, overlay state, and rendering
- `src/application/service`: use-case orchestration and outbound-port ownership
- `src/application/service/planning`: planning feature facades exposed to the TUI
- `src/application/port/outbound`: boundaries for app-server, filesystem, and worker execution
- `src/adapter/outbound`: concrete adapters grouped by infrastructure boundary such as app-server, DB, filesystem, and GitHub
- `src/domain`: UI-neutral models and invariants

## Small-Context Rules

- A feature change should usually start from one façade or entrypoint, not from a flat directory full of unrelated adapters.
- Infrastructure adapters should be skippable when tracing operator-visible behavior; they are implementation detail, not the main narrative.
- Files approaching roughly 800 LOC, or files mixing storage, recovery, rendering, and policy concerns, should be split by boundary in the same refactor campaign.
- Composition roots may wire concrete adapters together, but feature logic should depend on ports or feature façades instead of leaf adapter modules.

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
- Outbound directory layout should make the storage boundary obvious at a glance, for example `outbound/db`, `outbound/github`, `outbound/filesystem`, and `outbound/app_server`.
