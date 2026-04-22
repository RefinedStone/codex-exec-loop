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
