# Codex Assumption To Capability Target Map

This document records where the current runtime still assumes `codex app-server` behavior and
translates those assumptions into the capability targets defined in
[23-terminal-agent-capability-boundary-and-session-contract.md](23-terminal-agent-capability-boundary-and-session-contract.md).

It exists so bridge research can consume one current-state audit instead of repeating the same
inventory in every transport or experiment note.
The prioritized implementation-facing follow-ups derived from this map live in
[26-capability-map-prioritized-seam-follow-ups.md](26-capability-map-prioritized-seam-follow-ups.md).

## Working Rules

- This is an audit of current coupling, not a promise that every target becomes a real provider API.
- One current seam may map to more than one capability target.
- Capability names should lead future boundary work; `CodexAppServerPort` now remains as a
  compatibility port while application services depend on split capability-owned ports.
- `SessionCatalog` remains optional even when the current Codex path happens to support it.

## Implementation Checkpoint

The first extraction pass has landed:

- `StartupProbePort`, `InteractiveTurnRuntimePort`, and `SessionCatalogPort` exist as separate
  application outbound ports.
- `CodexAppServerPort` still exists, but only as a Codex-shaped compatibility adapter surface that
  delegates to those capability-owned ports.
- `TerminalBridgeAttachmentProfile` carries attachment mode and recovery-anchor truth through
  startup diagnostics and conversation events.
- `ConversationRuntimeControlTruth` records approval and interrupt support as runtime-native,
  manual-handoff, or unsupported.
- `SessionCatalog` now carries explicit `AttachOnly`, `HandleBasedReattach`, and
  `ProviderBackedCatalog` tiers, including unsupported and partial states.

## Mapping Table

| Current seam | Current Codex-only assumption | Capability target | Boundary note |
| --- | --- | --- | --- |
| `src/application/port/outbound/codex_app_server_port.rs`, `src/application/port/outbound/startup_probe_port.rs`, `src/application/port/outbound/session_catalog_port.rs`, `src/application/port/outbound/interactive_turn_runtime_port.rs`, `src/application/service/startup_service.rs`, `src/application/service/session_service.rs`, `src/application/service/conversation_service.rs` | the concrete Codex adapter can still satisfy startup checks, session listing, conversation snapshot loading, new-thread launch, and turn resume together | `StartupProbe`, `InteractiveTurnRuntime`, optional `SessionCatalog` | capability-owned ports are now split; remaining work is to keep new services from depending on the compatibility port directly |
| `src/application/service/startup_service.rs`, `src/domain/startup_diagnostics.rs`, `src/adapter/inbound/tui/app/shell_presentation/capability_copy.rs`, `src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_copy.rs` | the current shipped startup path still checks `codex` on `PATH`, app-server initialize, account state, and cwd | `StartupProbe` | startup now carries attachment profile truth, but local prerequisites and copy still need to be generalized before non-Codex paths |
| `src/application/service/session_service.rs`, `src/adapter/inbound/tui/app/shell_presentation/session_browser.rs`, `src/domain/recent_sessions.rs`, `src/domain/session_browser.rs`, `src/domain/session_summary.rs` | the Codex path still returns provider-thread listing and durable thread ids when available | optional `SessionCatalog` | session catalog tiering now models attach-only, handle-based, and provider-backed states; remaining work is to feed real non-provider catalog states from future adapters |
| `src/application/service/conversation_service.rs`, `src/application/port/outbound/interactive_turn_runtime_port.rs`, `src/adapter/outbound/app_server/mod.rs` | the current Codex implementation still starts or resumes a provider thread, starts a provider turn, and streams completion from explicit app-server events | `InteractiveTurnRuntime` | prompt submission, incremental output, completion summary, and control-truth reporting are capability-shaped; provider thread ids and turn ids stay metadata |
| `src/adapter/outbound/app_server/mod.rs`, `src/adapter/inbound/tui/conversation_text.rs`, `src/adapter/inbound/tui/app/conversation_runtime.rs`, `src/domain/conversation.rs` | app-server approval and interrupt limits are still Codex-specific behavior | `InteractiveTurnRuntime`, `TerminalBridgeAttachment` | approval and interrupt truth are now explicit as runtime-native, manual-handoff, or unsupported; remaining work is deeper operator flow for those states |
| `src/domain/terminal_bridge_attachment.rs`, `src/adapter/outbound/app_server/mod.rs`, `src/adapter/outbound/app_server/runtime.rs` | the current runtime still uses provider launch and provider reattach profiles backed by app-server threads | `TerminalBridgeAttachment` | mode and recovery-anchor vocabulary exists; future bridge work must add real local, wrapper, remote, or proxy profiles instead of adding more Codex-specific branching |
| `src/adapter/inbound/tui/app/shell_presentation/capability_copy.rs`, `src/adapter/inbound/tui/app/shell_presentation/session_browser.rs`, `src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_copy.rs` | some operator copy still names `codex app-server`, `codex shell`, and `codex binary` where the surface is intentionally Codex-only | `StartupProbe`, `InteractiveTurnRuntime`, optional `SessionCatalog`, `TerminalBridgeAttachment` | capability-shaped copy helpers exist; continue moving new wording through them instead of scattering provider names |

## Capability-Specific Targets

### `StartupProbe`

Current Codex-shaped state:

- `StartupService` depends on `StartupProbePort`
- the shipped startup checks still include `which codex`, app-server initialize, and account-read
  success
- diagnostics now include `TerminalBridgeAttachmentProfile`, while some shell copy still names the
  Codex path directly

Target:

- check the concrete launch or attach prerequisite for the chosen path
- report auth posture only when it exists for that path
- keep workspace and local-environment checks separate from provider-specific startup

### `InteractiveTurnRuntime`

Current Codex-shaped state:

- `ConversationService` depends on `InteractiveTurnRuntimePort`
- the current adapter still assumes explicit thread creation or resume, explicit turn ids, and
  server-driven turn stream events
- approval and interrupt support are exposed through `ConversationRuntimeControlTruth`

Target:

- own prompt submission, output observation, completion detection, and interrupt truth
- allow provider-native or terminal-derived turn completion signals
- keep approval behavior honest: runtime-native, manual handoff, or unsupported

### `SessionCatalog`

Current Codex-shaped state:

- `SessionService` depends on optional `SessionCatalogPort`
- `SessionCatalogTier` already names attach-only, handle-based reattach, and provider-backed
  catalog states
- the Codex adapter still supplies the provider-backed catalog path

Target:

- keep session listing optional
- support explicit tiers: attach-only, handle-based reattach, or provider-backed catalog
- let session UI explain unsupported or partial catalog behavior without implying a startup failure

### `TerminalBridgeAttachment`

Current Codex-shaped state:

- `TerminalBridgeAttachmentProfile` carries mode and recovery anchor through startup and stream
  events
- the current adapter only emits Codex provider-launch and provider-reattach profiles

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
- [26-capability-map-prioritized-seam-follow-ups.md](26-capability-map-prioritized-seam-follow-ups.md)
  turns this inventory into the ordered refactor slices that should land next.
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
