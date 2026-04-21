# Terminal Agent Bridge Research And Capability Boundary

This document is the active short and mid-term roadmap for exploring Claude Code class terminal
agents in Akra.

It is research-first.
No provider-specific adapter is implied by this document alone.

## Objective

Find an elegant way for Akra to work with non-Codex terminal agents while preserving the product's
terminal-first nature.

The output of this cycle should be:

- one comparison matrix
- one primary path
- one fallback path
- one capability boundary note

## Constraints

- Akra currently assumes `codex app-server` in multiple runtime seams.
- External terminal agents may expose only TTY interaction instead of a stable session API.
- Streaming fidelity, interrupt behavior, and approval handling matter more than transport novelty.
- The operator may be allowed to pre-open a terminal or tmux pane before Akra attaches.
- Security and recovery matter; local elegance wins over remote complexity unless research proves
  otherwise.

## Evaluation Rubric

Every candidate path should be scored against the same criteria:

| Criterion | What it means here |
| --- | --- |
| UX fit | whether the operator flow still feels like one terminal cockpit |
| streaming fidelity | whether token output, prompts, and terminal state can be mirrored cleanly |
| interrupt handling | whether stop or cancel signals can be issued predictably |
| approval handling | whether confirmation prompts can be surfaced or handed back safely |
| recovery | whether reconnect or restart rules are explainable |
| security | whether credentials, shell access, and transport scope stay understandable |
| portability | whether the path can work across the platforms Akra cares about |
| architecture fit | whether the path maps to capability seams without a fake shared API |

## Candidate Paths

### 1. Pre-Opened Local Terminal Attach

Attach to a user-prepared local PTY or tmux pane that already runs the target agent CLI.

Why it is attractive:

- preserves the true terminal behavior of the external agent
- minimizes invented protocol surface
- keeps the operator in control of the agent's startup and authentication state
- fits a terminal-native product story

Risks:

- attach semantics vary across shells and multiplexers
- session discovery may be partial or operator-assisted
- approval prompts may need careful handoff rules

### 2. Controlled Local Wrapper

Launch the external CLI inside a managed PTY and normalize the stream, interrupt, and lifecycle
events for Akra.

Why it is attractive:

- gives Akra more control over process lifecycle
- can standardize stream parsing and recovery more tightly
- works even when the operator does not pre-open a pane

Risks:

- wrapper behavior can drift from real CLI behavior
- approval and terminal UI quirks may become Akra's burden
- the wrapper can become an accidental pseudo-provider layer

### 3. SSH Or Tunnel Attach

Run the target agent on another host or forwarded endpoint and bridge that terminal back into Akra.

Why it is worth researching:

- could support remote workstations or isolated environments
- may be useful when the target CLI must run elsewhere

Why it is not the default:

- transport, auth, and recovery complexity grows fast
- operator mental model gets wider
- debugging becomes harder before the local boundary is even stable

### 4. Proxy Or Vibeproxy-Style Mediation

Insert a proxy layer between Akra and the terminal agent to normalize streams or commands.

Why it is worth researching:

- may offer stronger normalization and replay control
- may help when the external agent has irregular terminal behavior

Why it is risky:

- it invents another system to trust and debug
- it can obscure the real provider behavior
- it increases security and protocol surface before the local product loop is mature

## Working Recommendation For This Cycle

Primary path to evaluate first:

- pre-opened local terminal attach through PTY or tmux oriented attachment

Fallback path to evaluate second:

- controlled local wrapper inside a managed PTY

Deferred until the local boundary is stable:

- SSH or tunnel mediation
- proxy or vibeproxy-style mediation

The rationale is straightforward: Akra is already a terminal-native product, and the most elegant
first extension is the one that stays closest to real terminal behavior.

## Capability Boundary Target

The bridge should be described in small capabilities instead of one giant provider abstraction.

### `InteractiveTurnRuntime`

Owns prompt submission, stream observation, interrupt requests, and turn completion summaries.

### `StartupProbe`

Owns readiness checks for binary presence, auth posture, attach viability, and required local
prerequisites.

### `SessionCatalog`

Owns session discovery or reattachment only when the provider can support it.
It must be optional.

### `TerminalBridgeAttachment`

Owns attach or launch semantics for PTY, tmux, or other terminal-bridge mechanisms.

## Short-Term Research Sequence

1. Audit Codex-only coupling in current runtime seams.
2. Build the comparison matrix using the rubric above.
3. Write the primary-path and fallback-path decision note.
4. Only then decide whether a limited local-attach spike is worth queueing.

## Acceptance Signals

- all four candidate families are evaluated with the same rubric
- the primary path and fallback path are explicit
- the reasons for deferring remote and proxy paths are written down
- the capability boundary can be explained without claiming a fake shared session model

## Non-Goals

- shipping a full Claude adapter in this cycle
- assuming other agents match `codex app-server` semantics
- starting with network proxy infrastructure
- committing to one provider-specific UX before the bridge boundary is credible

## Related Docs

- [20-context-first-architecture-and-doc-coherence.md](20-context-first-architecture-and-doc-coherence.md)
- [17-structure-and-architecture-debt-map.md](17-structure-and-architecture-debt-map.md)
