# Context-First Architecture And Doc Coherence

This document is the active short and mid-term roadmap for making Akra easier to change with small
local context.

It is future-facing. Shipped behavior still lives in `docs/design/`.

## Objective

The next cycle should make one flow understandable without loading half the repository into memory.

That means:

- clearer doc entrypoints
- smaller ownership boundaries
- less mixed responsibility in hotspot files
- stable operator vocabulary across product, docs, and planning artifacts

## Working Rules

- `docs/design/` stays reserved for shipped truth.
- `docs/plan/` holds future work, research, and sequencing.
- `docs/supersession/` stays available as history, not as the active roadmap.
- Refactors must justify one operator-visible payoff, not just aesthetic cleanup.
- Capability boundaries come before provider-wide abstractions.

## Short-Term Plan

### 1. Normalize Entry Docs And Vocabulary

Refresh the repo entrypoints so a contributor can answer three questions quickly:

1. What is the current product contract?
2. What is the current roadmap?
3. Which doc is history only?

The shared vocabulary for this cycle should stay stable across README, docs/README, queue-idle
guidance, and roadmap docs:

- direction
- queue task
- proposed task
- accepted planning
- queue-idle policy
- repair
- capability boundary

### 2. Freeze Hotspot Split Order

The current debt map already names the right hotspots. The next step is to turn that map into an
ordered execution rule so refactor work does not thrash.

Current target order:

1. `src/adapter/inbound/tui/app/shell_presentation.rs` and nearby rendering or projection files
2. `src/adapter/inbound/tui/app/conversation_runtime.rs`
3. `src/adapter/inbound/tui/app/planning/controller.rs`
4. `src/application/service/parallel_mode_service.rs`

The rule is simple: do not begin a later hotspot slice without first recording why the earlier one
was skipped.

### 3. Audit Codex-Only Coupling

Before any external terminal-agent work, record where Codex assumptions are embedded today.

The audit should cover:

- `CodexAppServerPort`
- app-server spawn and readiness flow
- session discovery and resume assumptions
- approval or interrupt semantics that currently assume Codex behavior
- startup diagnostics that currently read as Codex-specific rather than capability-specific

The target is not a giant `Provider` abstraction. The target is a smaller set of capability notes
that future implementations can satisfy selectively.

### 4. Keep Planning Artifacts Clean

`directions.toml` should carry only live strategy.
Completed directions should leave the active map.
Future cycle details should sit in supporting docs and task-ledger items, not in a pile of done
directions.

## Mid-Term Implementation Track

### Boundary Theme A: Presentation Versus Layout

Separate wording, status projection, and operator-facing summaries from layout and rendering code so
shell UX changes stop dragging large file context behind them.

### Boundary Theme B: Conversation Lifecycle Versus Automation Lifecycle

Conversation flow, auto-follow logic, and shell state should stop competing in the same runtime
surface.

### Boundary Theme C: Planning Authoring Versus Planning Runtime

Planning setup, editing, validation, and repair should read as one bounded authoring story instead
of leaking across multiple runtime-shaped services.

### Boundary Theme D: Capability-First Runtime Seams

Where the product is currently Codex-shaped, define narrower capability seams first.
That work sets up terminal-agent exploration without overpromising multi-provider support.

## Acceptance Signals

- README and docs/README point to one active roadmap instead of scattering current intent across
  supersession history.
- A contributor can trace one roadmap item into the relevant code with one roadmap doc plus one
  current-truth doc.
- The hotspot order is explicit enough that future PRs can say why they chose one slice first.
- Docs/design is not polluted with future-state promises.
- Codex-specific assumptions are documented in capability terms before bridge research turns into
  implementation.

## Non-Goals

- broad renames with no operator-visible payoff
- architecture cleanup that does not reduce context fan-in
- rewriting current-truth docs to describe planned behavior
- introducing a monolithic provider abstraction as the first step

## Related Docs

- [17-structure-and-architecture-debt-map.md](17-structure-and-architecture-debt-map.md)
- [16-planning-and-automation-evolution.md](16-planning-and-automation-evolution.md)
- [14-product-elevation-blueprint.md](14-product-elevation-blueprint.md)
