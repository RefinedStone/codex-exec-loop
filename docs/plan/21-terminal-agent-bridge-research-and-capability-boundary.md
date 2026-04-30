# Terminal Agent Bridge Research And Capability Boundary

This document is the research hub for future non-Codex terminal-agent work in Akra.
It is a baseline, not an active rollout plan.

## Objective

Preserve a credible capability-first boundary for non-Codex work without pretending those agents
are drop-in replacements for `codex app-server`.

## Current Product Baseline

- main interactive runtime stays on `codex app-server`
- the next bounded expansion is a Claude-first headless runner for `PlanningWorkerPort` and future
  sub-task flows
- main interactive Claude, SSH or tunnel mediation, and proxy mediation remain out of scope

## Constraints

- Akra still ships around `codex app-server` for startup, session browsing, and the main
  conversation runtime.
- The current code already has capability-owned ports and domain truth types for startup probing,
  interactive turns, optional session catalog state, conversation control support, and terminal
  attachment profile.
- Future non-Codex work may expose only terminal or headless CLI behavior instead of a stable
  provider session API.
- Streaming fidelity, changed-file reporting, failure truth, and recovery vocabulary matter more
  than transport novelty.
- Capability seams must stay explicit so new work does not drift back into a fake universal
  provider contract.

## Document Set

Read the bridge work in this order:

1. This hub for the current baseline and reading order.
2. [25-codex-assumption-to-capability-target-map.md](25-codex-assumption-to-capability-target-map.md)
   for the Codex-shaped seams that already need capability-first names.
3. [22-terminal-agent-transport-and-attachment-matrix.md](22-terminal-agent-transport-and-attachment-matrix.md)
   for local attach, managed launch, SSH or tunnel, and proxy transport comparison.
4. [23-terminal-agent-capability-boundary-and-session-contract.md](23-terminal-agent-capability-boundary-and-session-contract.md)
   for capability seams and session expectations.
5. [24-terminal-agent-bridge-experiment-matrix.md](24-terminal-agent-bridge-experiment-matrix.md)
   for future experiment design and evidence targets.
6. [26-capability-map-prioritized-seam-follow-ups.md](26-capability-map-prioritized-seam-follow-ups.md)
   for the seam order that implementation work should consume instead of replacing.

## Current Report Status

- The hub, transport matrix, capability note, experiment matrix, assumption map, and seam-priority
  note exist as `21` through `26`.
- The main interactive runtime has been reset to a single `codex app-server` path.
- The first capability extraction pass has landed; remaining bridge research should build on the
  existing Rust seams instead of re-planning them.
- The remaining implementation gap is no longer a tmux bridge slice.
  It is the planning-worker-only Claude headless runner direction.

## Feasibility Judgment

| Scope | Status | Reason |
| --- | --- | --- |
| research and planning set | feasible now | the baseline docs cover transport comparison, capability vocabulary, experiment framing, and the implemented extraction checkpoint |
| Claude-first headless planning runner | feasible now as the next bounded slice | `PlanningWorkerPort` already exists and currently has a single app-server-backed implementation |
| managed launch or wrapper semantics | conditionally feasible | they fit a headless worker better than a new interactive runtime; attachment truth has vocabulary, but no non-Codex profile is wired yet |
| main interactive Claude runtime | deferred | changing the primary conversation runtime is a wider product decision than the next planning-worker slice |
| SSH or tunnel attach | deferred | auth, recovery, and portability costs are not justified by the current scope |
| proxy or vibeProxy-style mediation | deferred | extra protocol and trust surface are not justified without a concrete local gap |

## Acceptance Signals

- `21` stays short and clearly names the shipped runtime baseline plus the next bounded expansion
- `22` compares candidate transport families without declaring a new shipped primary path
- `23` explains `InteractiveTurnRuntime`, `StartupProbe`, `SessionCatalog`, and
  `TerminalBridgeAttachment` without pretending every provider shares Codex sessions
- `24` describes concrete future experiments for headless runner work and deferred-path evidence
  gaps
- `25` and `26` remain the implementation boundary input for future runner work

## Non-Goals

- shipping a main interactive Claude runtime in this cycle
- reviving `tmux` as a hidden or alternate product path
- starting with network proxy infrastructure
- assuming other agents match `codex app-server` semantics

## Related Docs

- [22-terminal-agent-transport-and-attachment-matrix.md](22-terminal-agent-transport-and-attachment-matrix.md)
- [23-terminal-agent-capability-boundary-and-session-contract.md](23-terminal-agent-capability-boundary-and-session-contract.md)
- [24-terminal-agent-bridge-experiment-matrix.md](24-terminal-agent-bridge-experiment-matrix.md)
- [25-codex-assumption-to-capability-target-map.md](25-codex-assumption-to-capability-target-map.md)
- [26-capability-map-prioritized-seam-follow-ups.md](26-capability-map-prioritized-seam-follow-ups.md)
- [20-context-first-architecture-and-doc-coherence.md](20-context-first-architecture-and-doc-coherence.md)
- [17-structure-and-architecture-debt-map.md](17-structure-and-architecture-debt-map.md)
