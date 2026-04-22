# Terminal Agent Bridge Research And Capability Boundary

This document is the active hub and decision note for exploring Claude Code class terminal agents
in Akra.

It is research-first.
No provider-specific adapter is implied by this document alone.

## Objective

Find an elegant way for Akra to work with non-Codex terminal agents while preserving the product's
terminal-first nature.

The output of this cycle should be:

- one hub doc that explains the decision and reading order
- one current-state audit that maps Codex-only assumptions into capability targets
- one transport and attachment matrix
- one capability-boundary note
- one experiment matrix
- one seam priority note
- one local tmux evidence package
- one explicit readiness-gate verdict

## Current Decision

Primary path:

- pre-opened local terminal attach, with tmux-oriented attachment as the concrete operator-ready
  shape

Fallback path:

- controlled local wrapper inside a managed PTY

Deferred until the local boundary is better evidenced:

- SSH or tunnel mediation
- proxy or vibeProxy-style mediation

The decision is intentionally conservative.
Akra is already terminal-native, so the first extension should stay close to real terminal behavior
and avoid inventing network or proxy infrastructure too early.

## Constraints

- Akra currently assumes `codex app-server` in multiple runtime seams.
- External terminal agents may expose only TTY interaction instead of a stable session API.
- Streaming fidelity, terminal-to-terminal data relay, interrupt behavior, and approval handling
  matter more than transport novelty.
- The operator may be allowed to pre-open a terminal or tmux pane before Akra attaches.
- Security and recovery matter; local elegance wins over remote complexity unless research proves
  otherwise.

## Document Set

Read the bridge work in this order:

1. This hub for the current decision and reading order.
2. [25-codex-assumption-to-capability-target-map.md](25-codex-assumption-to-capability-target-map.md)
   for the current Codex-shaped seams that need capability-first names.
3. [22-terminal-agent-transport-and-attachment-matrix.md](22-terminal-agent-transport-and-attachment-matrix.md)
   for tmux, PTY, wrapper, SSH, and proxy or vibeProxy-style transport comparison.
4. [23-terminal-agent-capability-boundary-and-session-contract.md](23-terminal-agent-capability-boundary-and-session-contract.md)
   for capability seams and session expectations.
5. [24-terminal-agent-bridge-experiment-matrix.md](24-terminal-agent-bridge-experiment-matrix.md)
   for actual experiment design and evidence targets.
6. [26-capability-map-prioritized-seam-follow-ups.md](26-capability-map-prioritized-seam-follow-ups.md)
   for the already-landed seam order that bridge implementation must consume instead of replacing.
7. [27-terminal-agent-tmux-local-attach-readiness-evidence.md](27-terminal-agent-tmux-local-attach-readiness-evidence.md)
   for executed tmux local-attach evidence.
8. [28-terminal-agent-tmux-local-attach-gate-verdict.md](28-terminal-agent-tmux-local-attach-gate-verdict.md)
   for the gate decision and implementation constraints.

## Short-Term Research Sequence

1. Audit Codex-only coupling in current runtime seams.
2. Compare terminal transport and attachment families with one rubric.
3. Lock the capability boundary without inventing a fake shared session API.
4. Write the experiments for local attach and managed wrapper.
5. Only then decide whether a limited local-attach spike is worth queueing.

## Current Report Status

- The hub, current-state audit, transport matrix, capability note, experiment matrix, and seam
  priority note now exist as `21` through `26`.
- The tmux local-attach evidence and gate verdict now exist as `27` and `28`.
- The primary path, fallback path, and deferred paths are now explicit instead of implied.
- The remaining gap is the first bounded tmux implementation slice, not missing research coverage.

## Feasibility Judgment

| Scope | Status | Reason |
| --- | --- | --- |
| research and planning set | feasible now | the document set already covers the hub, comparison rubric, capability boundary, and experiment checklist |
| tmux-oriented local attach spike | feasible now as a bounded next slice | the Codex capability audit, seam cleanup, and local evidence now cover relay fidelity, interrupt behavior, approval handling, recovery anchors, and operator setup costs well enough to start one tmux-only implementation path |
| managed local wrapper fallback | conditionally feasible | lifecycle control is strong, but realism costs must stay explicit |
| SSH or tunnel attach | deferred | remote auth, recovery, and portability costs are not yet justified by evidence |
| proxy or vibeProxy-style mediation | deferred | extra protocol and trust surface are not justified without a concrete local gap |

## Acceptance Signals

- `21` stays short and clearly names primary, fallback, and deferred paths
- `25` maps current `CodexAppServerPort`, startup checks, session catalog assumptions,
  approval or interrupt handling, and shell copy into capability targets
- `22` compares tmux, PTY, wrapper, SSH or tunnel, and proxy or vibeProxy-style mediation with
  one rubric
- `23` explains `InteractiveTurnRuntime`, `StartupProbe`, `SessionCatalog`, and
  `TerminalBridgeAttachment` without pretending every provider shares Codex sessions
- `24` describes concrete experiments for local attach, managed wrapper, and deferred-path evidence
  gaps
- `26` records the prioritized seam follow-ups for bridge implementation
- `27` records the executed tmux local-attach evidence with artifact links
- `28` records the readiness-gate verdict and the constraints for the first real implementation
  slice

## Non-Goals

- shipping a full Claude adapter in this cycle
- assuming other agents match `codex app-server` semantics
- starting with network proxy infrastructure
- committing to one provider-specific UX before the bridge boundary is credible

## Related Docs

- [22-terminal-agent-transport-and-attachment-matrix.md](22-terminal-agent-transport-and-attachment-matrix.md)
- [23-terminal-agent-capability-boundary-and-session-contract.md](23-terminal-agent-capability-boundary-and-session-contract.md)
- [24-terminal-agent-bridge-experiment-matrix.md](24-terminal-agent-bridge-experiment-matrix.md)
- [25-codex-assumption-to-capability-target-map.md](25-codex-assumption-to-capability-target-map.md)
- [26-capability-map-prioritized-seam-follow-ups.md](26-capability-map-prioritized-seam-follow-ups.md)
- [27-terminal-agent-tmux-local-attach-readiness-evidence.md](27-terminal-agent-tmux-local-attach-readiness-evidence.md)
- [28-terminal-agent-tmux-local-attach-gate-verdict.md](28-terminal-agent-tmux-local-attach-gate-verdict.md)
- [20-context-first-architecture-and-doc-coherence.md](20-context-first-architecture-and-doc-coherence.md)
- [17-structure-and-architecture-debt-map.md](17-structure-and-architecture-debt-map.md)
