# Codex Assumption To Capability Target Map

This document records where the current runtime still assumes `codex app-server` behavior and
translates those assumptions into the capability targets defined in
[23-terminal-agent-capability-boundary-and-session-contract.md](23-terminal-agent-capability-boundary-and-session-contract.md).

It exists so bridge research can consume one current-state audit instead of repeating the same
inventory in every transport or experiment note.

## Working Rules

- This is an audit of current coupling, not a promise that every target becomes a real provider API.
- One current seam may map to more than one capability target.
- Capability names should lead future boundary work even when the current Rust type names still say
  `Codex`.
- `SessionCatalog` remains optional even when the current Codex path happens to support it.

## Mapping Table

| Current seam | Current Codex-only assumption | Capability target | Boundary note |
| --- | --- | --- | --- |
| `src/application/port/outbound/codex_app_server_port.rs`, `src/application/service/startup_service.rs`, `src/application/service/session_service.rs`, `src/application/service/conversation_service.rs` | one outbound port can cover startup checks, session listing, conversation snapshot loading, new-thread launch, and turn resume through one `codex app-server` model | `StartupProbe`, `InteractiveTurnRuntime`, optional `SessionCatalog` | future seams should split by capability ownership instead of cloning one universal app-server-shaped port |
| `src/application/service/startup_service.rs`, `src/domain/startup_diagnostics.rs`, `src/adapter/inbound/tui/app/shell_presentation.rs`, `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/base.rs`, `src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_copy.rs` | startup readiness means `codex` is on `PATH`, the app-server initialize call succeeded, and account state was read | `StartupProbe` | the probe should generalize to launch-target presence, attach viability, auth posture, and local prerequisites without assuming launch and attach are the same path |
| `src/application/service/session_service.rs`, `src/adapter/inbound/tui/app/shell_presentation/session_browser.rs`, `src/domain/session_summary.rs` | recent-session discovery is a baseline feature backed by provider-thread listing and durable thread ids | optional `SessionCatalog` | session discovery should degrade to attach-only or handle-based reattach without making the whole bridge look broken |
| `src/application/service/conversation_service.rs`, `src/adapter/outbound/app_server/mod.rs` | turn execution always means start or resume a provider thread, start a provider turn, and stream completion from explicit app-server events | `InteractiveTurnRuntime` | the mandatory contract is prompt submission, incremental output, completion summary, and truthful interrupt support; provider thread ids and turn ids are metadata, not the baseline boundary |
| `src/adapter/outbound/app_server/mod.rs`, `src/adapter/inbound/tui/conversation_text.rs`, `src/adapter/inbound/tui/app/conversation_runtime.rs` | approval is effectively disabled or handed back manually because the app-server protocol does not expose client approve or deny actions, and stop or interrupt behavior is not modeled as an explicit capability | `InteractiveTurnRuntime` plus `TerminalBridgeAttachment` | future runtime notes should say whether approval and interrupt are runtime-native, manual handoff, or unsupported instead of silently inheriting Codex behavior |
| `src/adapter/outbound/app_server/mod.rs`, `src/adapter/outbound/app_server/runtime.rs` | launch, reconnect, and resume are all variations of one app-server connection story, so no separate attachment mode or recovery anchor needs to be surfaced | `TerminalBridgeAttachment` | non-Codex terminal paths need explicit mode and anchor vocabulary such as local wrapper handle, tmux pane id, SSH target, or proxy session id |
| `src/adapter/inbound/tui/app/shell_presentation/shell_copy.rs`, `src/adapter/inbound/tui/app/shell_presentation/session_browser.rs`, `src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_copy.rs` | operator copy can name `codex app-server`, `codex shell`, and `codex binary` as if every future path exposes the same substrate | capability-first operator vocabulary from `StartupProbe`, `InteractiveTurnRuntime`, `SessionCatalog`, and `TerminalBridgeAttachment` | keep Codex-specific wording only where the surface is intentionally Codex-only; otherwise prefer launch target, bridge readiness, session catalog, conversation history, approval handoff, and recovery anchor language |

## Capability-Specific Targets

### `StartupProbe`

Current Codex-shaped state:

- startup checks are hard-wired to `which codex`, app-server initialize, and account-read success
- diagnostics fields and shell copy still encode that specific sequence directly

Target:

- check the concrete launch or attach prerequisite for the chosen path
- report auth posture only when it exists for that path
- keep workspace and local-environment checks separate from provider-specific startup

### `InteractiveTurnRuntime`

Current Codex-shaped state:

- the runtime assumes explicit thread creation or resume, explicit turn ids, and server-driven turn
  stream events
- approval review wording is inherited from current app-server protocol limits rather than an
  explicit capability contract

Target:

- own prompt submission, output observation, completion detection, and interrupt truth
- allow provider-native or terminal-derived turn completion signals
- keep approval behavior honest: runtime-native, manual handoff, or unsupported

### `SessionCatalog`

Current Codex-shaped state:

- recent sessions are treated as a normal shell feature instead of one optional capability tier
- the session overlay reads provider session metadata as if that inventory always exists

Target:

- keep session listing optional
- support explicit tiers: attach-only, handle-based reattach, or provider-backed catalog
- let session UI explain unsupported or partial catalog behavior without implying a startup failure

### `TerminalBridgeAttachment`

Current Codex-shaped state:

- the current adapter hides launch, reconnect, and recovery inside the app-server runtime
- no explicit attachment-mode label or recovery anchor is carried through the shell model

Target:

- name whether the path is launch-only, local attach, managed wrapper, remote attach, or
  proxy-mediated
- expose the anchor Akra can store for recovery
- keep transport and attachment truth separate from turn-runtime semantics

## Immediate Consumers

- [20-context-first-architecture-and-doc-coherence.md](20-context-first-architecture-and-doc-coherence.md)
  uses this as the current audit requested before external bridge work.
- [21-terminal-agent-bridge-research-and-capability-boundary.md](21-terminal-agent-bridge-research-and-capability-boundary.md)
  uses this as the current-state input for the research set.
- [22-terminal-agent-transport-and-attachment-matrix.md](22-terminal-agent-transport-and-attachment-matrix.md)
  should read the attachment rows here before comparing tmux, wrapper, SSH, or proxy paths.
- [23-terminal-agent-capability-boundary-and-session-contract.md](23-terminal-agent-capability-boundary-and-session-contract.md)
  defines the target boundary names that this audit maps to.
- [24-terminal-agent-bridge-experiment-matrix.md](24-terminal-agent-bridge-experiment-matrix.md)
  should declare which mapped capabilities each experiment really satisfies.

## Non-Goals

- renaming every Codex-shaped type in one pass
- inventing a universal provider session model
- claiming non-Codex support is implementation-ready
- hiding real provider limits behind vague abstraction language
