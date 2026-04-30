# Capability Map Prioritized Seam Follow-Ups

This document turns the current Codex-assumption audit into the next implementation-facing seam
order.

It is not a transport experiment plan.
It is the prioritized follow-up list that should shape refactor slices before terminal-agent bridge
work tries to jump into adapter implementation.

The first capability extraction pass has already landed. This sequence now separates implemented
checkpoint slices from remaining hardening work so future refactors do not repeat the completed
domain and port extraction.

Completed checkpoint:

- application services depend on `StartupProbePort`, `InteractiveTurnRuntimePort`, and
  `SessionCatalogPort`
- `CodexAppServerPort` remains as a compatibility port for the current Codex adapter
- shell presentation has capability copy helpers for startup, session catalog tier, and attachment
  profile wording
- domain types now model session catalog tiers, conversation control truth, and terminal attachment
  mode plus recovery anchor

## Prioritization Rules

- respect the hotspot order in [17-structure-and-architecture-debt-map.md](17-structure-and-architecture-debt-map.md)
  unless a slice explicitly records why an earlier hotspot was skipped
- prefer operator-visible wording and projection cleanup before deep outbound boundary work
- split capability ownership before inventing any shared multi-provider runtime layer
- keep `SessionCatalog` optional throughout the sequence
- do not let tmux, wrapper, SSH, or proxy transport details leak into early capability cleanup

## Remaining Priority Order

| Priority | Seam follow-up | Capability target | Primary files or modules | Why now | Exit signal |
| --- | --- | --- | --- | --- | --- |
| 1 | shell capability wording hardening | `StartupProbe`, `InteractiveTurnRuntime`, `SessionCatalog`, `TerminalBridgeAttachment` | `src/adapter/inbound/tui/app/shell_presentation/capability_copy.rs`, `src/adapter/inbound/tui/app/shell_presentation/session_browser.rs`, `src/adapter/inbound/tui/app/shell_presentation/status_panels/*`, `src/adapter/inbound/tui/conversation_text.rs` | the helper boundary exists, but new copy must keep provider names intentional and capability facts centralized | Codex-specific wording only appears in Codex-only loading or adapter-specific states, and capability summaries flow through the helper modules |
| 2 | compatibility-port containment | `StartupProbe`, `InteractiveTurnRuntime`, optional `SessionCatalog` | `src/application/port/outbound/codex_app_server_port.rs`, `src/application/port/outbound/*_port.rs`, `src/adapter/outbound/app_server/mod.rs`, composition roots | application services now use smaller ports, so the remaining risk is accidental new dependency on the compatibility port | new application services and tests type against capability-owned ports unless they are explicitly testing the Codex compatibility bridge |
| 3 | non-provider session catalog inputs | optional `SessionCatalog` | `src/application/service/session_service.rs`, `src/adapter/inbound/tui/app/shell_runtime.rs`, `src/adapter/inbound/tui/app/shell_presentation/session_browser.rs`, `src/domain/recent_sessions.rs` | the domain and UI can render unsupported, partial, handle-based, and provider-backed tiers, but only the Codex provider-backed adapter currently feeds real data | future attach-only or handle-based adapters can return truthful catalog states without changing the shell surface |
| 4 | approval and interrupt operator flow | `InteractiveTurnRuntime`, `TerminalBridgeAttachment` | `src/adapter/inbound/tui/conversation_text.rs`, `src/adapter/inbound/tui/app/conversation_runtime.rs`, `src/adapter/inbound/tui/app/conversation_model/view_model.rs`, `src/adapter/outbound/app_server/mod.rs` | control truth is modeled, but the operator flow still needs deeper handling for manual-handoff and unsupported states | the shell can state and route runtime-native, manual-handoff, or unsupported behavior for approval and interrupt without pretending all bridges behave like Codex |
| 5 | real non-Codex attachment profiles | `TerminalBridgeAttachment`, `StartupProbe` | `src/domain/terminal_bridge_attachment.rs`, `src/adapter/outbound/app_server/runtime.rs`, future bridge adapters | the domain vocabulary exists, but only provider launch and provider reattach are currently emitted | a future tmux-oriented attach or managed wrapper path can emit its own mode, recovery anchor, and control limits without app-server-specific branching |

## Follow-Up Notes By Slice

### 1. Shell Capability Wording Hardening

Operator payoff:

- startup, session, and live-tail surfaces stop implying every supported path is a Codex app-server
- future bridge work keeps one place to update wording instead of scattered string edits

Guardrails:

- do not rename every current Rust type just to match future wording
- keep the slice focused on copy, projection helpers, and shell-facing status assembly

### 2. Compatibility-Port Containment

Operator payoff:

- startup, sessions, and turns become easier to reason about separately
- non-Codex work can satisfy only the capabilities it really has

Guardrails:

- current Codex adapter may continue to implement all split ports in one adapter
- avoid a monolithic `ProviderPort` replacement
- avoid adding new application-service dependencies on `CodexAppServerPort`

### 3. Non-Provider Session Catalog Inputs

Operator payoff:

- attach-only paths can still feel deliberate rather than degraded or broken
- reattach expectations become explicit instead of silently inherited from provider-backed sessions

Guardrails:

- do not make session listing a startup prerequisite
- keep session tier language aligned with
  [23-terminal-agent-capability-boundary-and-session-contract.md](23-terminal-agent-capability-boundary-and-session-contract.md)

### 4. Approval And Interrupt Operator Flow

Operator payoff:

- the shell can explain what the current path really supports before the operator gets stuck
- bridge experiments can report manual-handoff or unsupported behavior honestly

Guardrails:

- do not promise interactive approve or deny actions before a real path supports them
- keep approval and interrupt truth separate from transport branding

### 5. Real Non-Codex Attachment Profiles

Operator payoff:

- future local-attach or wrapper flows can explain how recovery works
- operator expectations stay grounded in real anchors such as pane id, wrapper handle, or process ownership

Guardrails:

- do not start with SSH or proxy-oriented abstractions
- keep the seam small enough that tmux-oriented attach and managed wrapper can satisfy it differently

## Deferred Until After This Sequence

- tmux-oriented attach spike work
- managed wrapper spike work
- SSH or tunnel transport work
- proxy or vibeProxy-style mediation
- broad renames that only replace `Codex` with generic nouns

## How To Use This Doc

- pick the highest slice that matches the current hotspot order
- if a later slice starts first, record why the earlier slice was skipped
- keep each implementation slice small enough to land as one reviewable commit and PR
- update the capability map only when the current-state coupling or implemented checkpoint materially
  changes

## Related Docs

- [17-structure-and-architecture-debt-map.md](17-structure-and-architecture-debt-map.md)
- [20-context-first-architecture-and-doc-coherence.md](20-context-first-architecture-and-doc-coherence.md)
- [21-terminal-agent-bridge-research-and-capability-boundary.md](21-terminal-agent-bridge-research-and-capability-boundary.md)
- [23-terminal-agent-capability-boundary-and-session-contract.md](23-terminal-agent-capability-boundary-and-session-contract.md)
- [25-codex-assumption-to-capability-target-map.md](25-codex-assumption-to-capability-target-map.md)
