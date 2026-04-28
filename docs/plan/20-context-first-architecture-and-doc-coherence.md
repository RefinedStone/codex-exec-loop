# Context-First Architecture And Doc Coherence

This document is the active short and mid-term roadmap for making Akra easier to change with small
local context.

It is future-facing. The current supersession, planning, and directions contract lives in
`docs/supersession/`.

## Objective

The next cycle should make one flow understandable without loading half the repository into memory.

That means:

- clearer doc entrypoints
- smaller ownership boundaries
- less mixed responsibility in hotspot files
- stable operator vocabulary across product, docs, and planning artifacts

## Working Rules

- `docs/supersession/current-contract.md` is the canonical current contract.
- `docs/supersession/remaining-work.md` holds unfinished or lightly validated work.
- `docs/design/` holds supporting deep dives and boundary explanations.
- `docs/plan/` holds future work, research, sequencing, and historical audits.
- Refactors must justify one operator-visible payoff, not just aesthetic cleanup.
- Capability boundaries come before provider-wide abstractions.

## Short-Term Plan

### 1. Normalize Entry Docs And Vocabulary

Keep the repo entrypoints aligned so a contributor can answer four questions quickly:

1. What is the current product contract?
2. What is still unfinished?
3. What is the active roadmap?
4. Which doc is history only?

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
The current audit output lives in
[25-codex-assumption-to-capability-target-map.md](25-codex-assumption-to-capability-target-map.md).
The prioritized seam order derived from that audit lives in
[26-capability-map-prioritized-seam-follow-ups.md](26-capability-map-prioritized-seam-follow-ups.md).
The terminal-agent research set in `docs/plan/21-*`, `docs/plan/22-*`, `docs/plan/23-*`, and
`docs/plan/24-*` should consume that audit rather than duplicate it.

### 4. Keep Planning Artifacts Clean

`directions.toml` should carry only live strategy.
Completed directions should leave the active map.
Future cycle details should sit in supporting docs and task-authority items, not in a pile of done
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

- README and docs/README point to one current-contract hub plus one active roadmap.
- A contributor can trace one roadmap item into the relevant code with one roadmap doc plus one
  current-truth doc.
- The hotspot order is explicit enough that future PRs can say why they chose one slice first.
- `docs/supersession/` stays compact for implemented behavior and comparatively detailed only for
  unfinished work.
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
- [21-terminal-agent-bridge-research-and-capability-boundary.md](21-terminal-agent-bridge-research-and-capability-boundary.md)
- [25-codex-assumption-to-capability-target-map.md](25-codex-assumption-to-capability-target-map.md)
- [26-capability-map-prioritized-seam-follow-ups.md](26-capability-map-prioritized-seam-follow-ups.md)
