# Terminal Agent Bridge Research And Capability Boundary

## Goal

Preserve the non-Codex terminal-agent research set as a capability and transport baseline without
implying that any bridge transport is the active shipped runtime path.

## Why This Direction Exists

Akra still ships around `codex app-server`, but future headless or non-Codex work needs stable
capability vocabulary and a bounded comparison set.
This direction keeps that baseline intact after the tmux-specific rollout is removed from the
official product path.

## Near-Term Focus

- turn `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md` into a hub and
  split transport, capability, and experiment detail into `docs/plan/22-*`,
  `docs/plan/23-*`, and `docs/plan/24-*`
- compare local attach, managed wrapper, SSH or tunnel, and proxy or vibeProxy-style mediation
  with one relay-and-control rubric
- keep the research descriptive instead of promoting tmux or any other bridge transport as the
  current product runtime
- keep managed launch or wrapper concepts available for future headless runner work without
  widening into a full bridge implementation plan here
- name the capability targets needed for future work: `InteractiveTurnRuntime`, `StartupProbe`,
  `SessionCatalog`, and `TerminalBridgeAttachment`

## Current Report Status

- `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md` now acts as the hub and
  current decision note.
- `docs/plan/22-terminal-agent-transport-and-attachment-matrix.md` records the common transport
  comparison rubric and candidate matrix.
- `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md` names the minimum
  capability boundary without pretending every provider looks like `codex app-server`.
- `docs/plan/24-terminal-agent-bridge-experiment-matrix.md` defines the evidence Akra still needs
  before a future non-Codex runner slice is justified.
- The remaining implementation gap sits under the active Claude-first headless runner direction,
  not under this research baseline.

## Detailed Research Plan

1. Keep the `21` through `24` document set stable as the research hub, matrix, capability note,
   and experiment plan.
2. Keep the Codex-only capability audit and seam notes as the implementation boundary input rather
   than reopening universal-provider abstraction work.
3. Keep the main interactive runtime on `codex app-server` while future work consumes the research
   set through `PlanningWorkerPort` or other bounded seams.
4. Treat local attach, managed launch, SSH or tunnel, and proxy mediation as comparison families,
   not as a default implementation queue.
5. Keep wider transport work out of scope until a future bounded runner slice proves a concrete
   reason to widen.

## Feasibility Judgment

- Research documentation: feasible now and materially in place.
- Claude-first headless runner design work: feasible now as the next bounded implementation slice.
- managed local wrapper or launch semantics: conditionally feasible, with explicit realism costs.
- SSH or tunnel and proxy or vibeProxy-style mediation: not justified at this stage.

## Acceptance

- the hub doc points to transport, capability, and experiment supporting docs with a clear reading
  order
- the transport matrix covers terminal-to-terminal input relay, output relay, interrupt behavior,
  approval handling, recovery, security, portability, and architecture fit
- candidate transport families stay explicit without claiming a new shipped primary path
- deferred paths have written reasons instead of silent omission
- the capability vocabulary stays stable enough that future headless runner work can consume it
  directly

## Supporting Docs

- `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`
- `docs/plan/22-terminal-agent-transport-and-attachment-matrix.md`
- `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`
- `docs/plan/24-terminal-agent-bridge-experiment-matrix.md`
- `docs/plan/25-codex-assumption-to-capability-target-map.md`
- `docs/plan/26-capability-map-prioritized-seam-follow-ups.md`
- `docs/design/04-hexagonal-runtime-architecture.md`
