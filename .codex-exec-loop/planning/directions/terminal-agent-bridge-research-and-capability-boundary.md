# Terminal Agent Bridge Research And Capability Boundary

## Goal

Define how Akra could speak to Claude Code class terminal agents without pretending they are
drop-in replacements for `codex app-server`.

The outcome of this direction is a credible research-backed boundary and document set, not a rushed
provider implementation.

## Why This Direction Exists

Akra is currently Codex-only in both runtime coupling and operator narrative.
That is acceptable for the shipped product, but it blocks elegant exploration of other terminal
agents unless the boundary is clarified first.

## Near-Term Focus

- turn `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md` into a hub and
  split transport, capability, and experiment detail into `docs/plan/22-*`,
  `docs/plan/23-*`, and `docs/plan/24-*`
- compare tmux, PTY, managed wrapper, SSH or tunnel, and proxy or vibeProxy-style mediation with
  one relay-and-control rubric
- prefer tmux-oriented local attach first, because it preserves terminal reality while giving Akra
  a stable operator-managed anchor
- keep the managed local wrapper as explicit fallback and keep SSH or tunnel plus proxy paths in
  deferred research unless local evidence reveals a real gap
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
  before any bounded bridge spike is justified.
- The remaining gap is no longer document structure. It is Codex-only boundary validation plus one
  local evidence pass.

## Detailed Research Plan

1. Keep the `21` through `24` document set stable as the research hub, matrix, capability note,
   and experiment plan.
2. Finish the Codex-only capability audit so bridge work targets product seams instead of provider
   vocabulary drift.
3. Collect local evidence in the same order already implied by the research set:
   - tmux-oriented local attach as the primary path
   - managed local wrapper as the fallback path
4. Re-evaluate whether a bounded local spike is justified only after relay fidelity, interrupt
   truthfulness, approval handling, recovery anchors, and operator setup costs are written down.
5. Keep SSH or tunnel and proxy or vibeProxy-style mediation deferred unless the local evidence
   exposes a concrete unsolved gap.

## Feasibility Judgment

- Research documentation: feasible now and materially in place.
- tmux-oriented local attach spike: conditionally feasible after the Codex capability audit and a
  documented local evidence pass.
- managed local wrapper fallback: conditionally feasible, with explicit realism costs.
- SSH or tunnel and proxy or vibeProxy-style mediation: not justified at this stage.

## Acceptance

- the hub doc points to transport, capability, and experiment supporting docs with a clear reading
  order
- the transport matrix covers terminal-to-terminal input relay, output relay, interrupt behavior,
  approval handling, recovery, security, portability, and architecture fit
- one primary bridge path and one fallback path are named explicitly
- deferred paths have written reasons instead of silent omission
- no provider-specific adapter work is promoted before the boundary and experiment notes are stable

## Supporting Docs

- `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`
- `docs/plan/22-terminal-agent-transport-and-attachment-matrix.md`
- `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`
- `docs/plan/24-terminal-agent-bridge-experiment-matrix.md`
- `docs/design/04-hexagonal-runtime-architecture.md`
