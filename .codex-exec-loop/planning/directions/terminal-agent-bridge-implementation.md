# Terminal Agent Bridge Implementation

## Goal

Take the completed terminal-agent bridge research set and turn it into bounded implementation work
without reopening the original research questions or inventing a universal provider abstraction.

## Why This Direction Exists

The repo already has the transport comparison, capability boundary, and seam-priority notes for
non-Codex terminal-agent work.
What remains is to convert that research into narrow executable slices.

The implementation direction exists so the queue can talk about real tmux-first work instead of
keeping execution tasks attached to the broader research label forever.

## Near-Term Focus

- finish the tmux local-attach readiness work needed before the first real adapter slice can start
- land one tmux-only implementation path that satisfies `StartupProbe`,
  `InteractiveTurnRuntime`, optional `SessionCatalog`, and `TerminalBridgeAttachment`
- keep approval, interrupt, attachment mode, and recovery anchor truth explicit in the shell
- leave managed wrapper, SSH or tunnel, and proxy or vibeProxy-style mediation out of scope until
  tmux proves a concrete need to widen

## Acceptance

- the queue no longer treats bridge execution as undifferentiated research work
- the tmux path is implemented through capability seams instead of a provider-wide bridge API
- operator-facing copy explains what tmux local attach can and cannot do before a turn starts
- fallback and deferred transports remain explicit rather than creeping into the first slice

## Supporting Docs

- `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`
- `docs/plan/22-terminal-agent-transport-and-attachment-matrix.md`
- `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`
- `docs/plan/24-terminal-agent-bridge-experiment-matrix.md`
- `docs/plan/25-codex-assumption-to-capability-target-map.md`
- `docs/plan/26-capability-map-prioritized-seam-follow-ups.md`
