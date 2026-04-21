# Terminal Agent Bridge Research And Capability Boundary

## Goal

Define how Akra could speak to Claude Code class terminal agents without pretending they are
drop-in replacements for `codex app-server`.

The outcome of this direction is a credible research-backed boundary, not a rushed provider
implementation.

## Why This Direction Exists

Akra is currently Codex-only in both runtime coupling and operator narrative.
That is acceptable for the shipped product, but it blocks elegant exploration of other terminal
agents unless the boundary is clarified first.

## Near-Term Focus

- compare local attach, local wrapper, SSH or tunnel, and proxy-style mediation with one rubric
- prefer local terminal attachment paths first, because they preserve terminal reality and reduce invented protocol surface
- name the capability targets needed for future work: `InteractiveTurnRuntime`, `StartupProbe`, `SessionCatalog`, and `TerminalBridgeAttachment`
- keep research in docs/plan until one primary path and one fallback path are chosen

## Acceptance

- the research matrix covers UX, streaming fidelity, interrupt behavior, approval handling, recovery, security, portability, and architecture fit
- one primary bridge path and one fallback path are named explicitly
- deferred paths have written reasons instead of silent omission
- no provider-specific adapter work is promoted before the boundary note is stable

## Supporting Docs

- `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`
- `docs/design/04-hexagonal-runtime-architecture.md`
