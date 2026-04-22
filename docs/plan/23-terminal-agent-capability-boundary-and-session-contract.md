# Terminal Agent Capability Boundary And Session Contract

This document defines the capability seams Akra should target before any non-Codex provider work
goes beyond research.

The point is not to recreate the `codex app-server` model with different names.
The point is to name the smallest capabilities that different terminal-agent paths can satisfy.
Use [25-codex-assumption-to-capability-target-map.md](25-codex-assumption-to-capability-target-map.md)
for the current Codex-shaped seam inventory; this document defines the target boundary names, not
the current implementation map.

## Boundary Principles

- prefer small capabilities over one giant provider interface
- do not assume every agent offers discoverable sessions, durable thread ids, or explicit turn
  lifecycle events
- treat session discovery as optional capability, not a baseline requirement
- keep terminal transport concerns separate from prompt or turn semantics
- let providers be partial: attach-only is still valid if the capability contract says so

## Capability Summary

| Capability | Required | Owns | Must not assume |
| --- | --- | --- | --- |
| `InteractiveTurnRuntime` | yes | prompt submission, output observation, interrupt requests, completion summary | stable session list or explicit provider turn ids |
| `StartupProbe` | yes | binary presence, auth posture, attach viability, required local prerequisites | that launch and attach are the same path |
| `SessionCatalog` | optional | session discovery, reattach metadata, operator-selectable handles | that every provider exposes queryable sessions |
| `TerminalBridgeAttachment` | yes for terminal-agent paths | PTY, tmux, wrapper, SSH, or proxy attach and launch semantics | that all attachment shapes expose the same control surface |

## Capability Contracts

### `InteractiveTurnRuntime`

Responsibilities:

- submit prompt content into the active terminal-agent path
- observe streaming output and terminal-state transitions that matter to Akra
- request interrupt or stop when the provider path supports it
- emit a completion summary even when the provider offers only terminal-derived completion signals

Minimum contract:

- Akra can send prompt text
- Akra can read output incrementally
- Akra can detect end-of-turn with provider-native or terminal-derived heuristics
- Akra can surface “interrupt supported / not supported” truthfully

What stays provider-specific:

- exact prompt framing
- turn-boundary detection details
- approval prompt signatures
- whether stop is a signal, control sequence, or unsupported

### `StartupProbe`

Responsibilities:

- confirm the target CLI or bridge dependency exists
- confirm auth posture is credible enough to proceed
- confirm the local attach prerequisites exist
- surface actionable failure reasons before the operator enters a broken bridge flow

Minimum contract:

- binary or endpoint presence check
- auth or readiness check when meaningful
- attach-target validation for tmux pane, wrapper launch, SSH target, or proxy endpoint

What stays provider-specific:

- auth prompt wording
- environment variable requirements
- target-specific reachability probes

### `SessionCatalog`

Responsibilities:

- list sessions only when the provider or bridge path exposes a stable session concept
- allow the operator to select a known handle for reattachment
- describe when catalog data is partial, stale, or unsupported

Minimum contract:

- capability can be absent without breaking the rest of the bridge design
- when present, each returned handle must be explicit about its type: provider session id, tmux
  pane id, wrapper handle, remote target, or proxy session id

What stays provider-specific:

- whether sessions are queryable at all
- how durable the handle is
- whether session history is inspectable or only attachable

### `TerminalBridgeAttachment`

Responsibilities:

- describe how Akra launches or attaches to the terminal-agent path
- expose the anchor used for later recovery
- surface whether the bridge is local attach, managed wrapper, remote attach, or proxy-mediated

Minimum contract:

- launch or attach entrypoint
- explicit attachment mode label
- stable recovery anchor when one exists
- truthful statement of what the attachment can and cannot control

What stays provider-specific:

- PTY allocation details
- tmux pane or session addressing
- SSH target semantics
- proxy endpoint and replay protocol details

## Session Contract Tiers

Not every provider path deserves the same session promises.

| Tier | Meaning | Example |
| --- | --- | --- |
| attach-only | Akra can attach or launch but cannot list prior sessions reliably | raw local PTY or minimal wrapper |
| handle-based reattach | Akra can rediscover a stable local handle | tmux pane or session attach |
| provider-backed catalog | Akra can query provider-native session metadata | future provider that exposes a real session API |

Akra should design for these tiers explicitly instead of claiming a universal session model.

## Boundary Outcome For This Cycle

- `InteractiveTurnRuntime` and `StartupProbe` are mandatory research targets.
- `TerminalBridgeAttachment` is mandatory because transport shape is the heart of the problem.
- `SessionCatalog` remains optional and must not block the local attach path.
- Any future spike should declare which tier it satisfies before code is written.

## Related Docs

- [25-codex-assumption-to-capability-target-map.md](25-codex-assumption-to-capability-target-map.md)
- [21-terminal-agent-bridge-research-and-capability-boundary.md](21-terminal-agent-bridge-research-and-capability-boundary.md)
- [22-terminal-agent-transport-and-attachment-matrix.md](22-terminal-agent-transport-and-attachment-matrix.md)
- [24-terminal-agent-bridge-experiment-matrix.md](24-terminal-agent-bridge-experiment-matrix.md)
