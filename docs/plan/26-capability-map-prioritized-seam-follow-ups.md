# Capability Map Prioritized Seam Follow-Ups

This document turns the current Codex-assumption audit into the next implementation-facing seam
order.

It is not a transport experiment plan.
It is the prioritized follow-up list that should shape refactor slices before terminal-agent bridge
work tries to jump into adapter implementation.

This sequence intentionally keeps the first three slices on shell, outbound-capability ownership,
and optional session catalog work even though older hotspot order also called out conversation
runtime. The capability audit showed those three seams must narrow first so later approval,
interrupt, and attachment truth work can land without inheriting the old app-server-shaped wording
and port boundary.

## Prioritization Rules

- respect the hotspot order in [17-structure-and-architecture-debt-map.md](17-structure-and-architecture-debt-map.md)
  unless a slice explicitly records why an earlier hotspot was skipped
- prefer operator-visible wording and projection cleanup before deep outbound boundary work
- split capability ownership before inventing any shared multi-provider runtime layer
- keep `SessionCatalog` optional throughout the sequence
- do not let tmux, wrapper, SSH, or proxy transport details leak into early capability cleanup

## Priority Order

| Priority | Seam follow-up | Capability target | Primary files or modules | Why now | Exit signal |
| --- | --- | --- | --- | --- | --- |
| 1 | shell capability wording and projection split | `StartupProbe`, `InteractiveTurnRuntime`, `SessionCatalog`, `TerminalBridgeAttachment` | `src/adapter/inbound/tui/app/shell_presentation.rs`, `src/adapter/inbound/tui/app/shell_presentation/shell_copy.rs`, `src/adapter/inbound/tui/app/shell_presentation/session_browser.rs`, `src/adapter/inbound/tui/app/shell_presentation/status_panels/*`, `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/base.rs` | current hotspot order already puts shell presentation first, and current copy still hard-codes `codex shell`, `codex binary`, and `codex app-server` where future capability-first wording should live | shell wording is projected from capability-shaped helpers and only names Codex where the surface is intentionally Codex-specific |
| 2 | outbound capability ownership split | `StartupProbe`, `InteractiveTurnRuntime`, optional `SessionCatalog` | `src/application/port/outbound/codex_app_server_port.rs`, `src/application/service/startup_service.rs`, `src/application/service/session_service.rs`, `src/application/service/conversation_service.rs`, `src/adapter/outbound/app_server/mod.rs` | the audit is currently trapped behind one app-server-shaped port, so future bridge work would otherwise inherit a fake universal provider seam | application services depend on smaller capability-owned ports while the current Codex adapter can still implement all of them together |
| 3 | optional session catalog and session-tier surface | optional `SessionCatalog` | `src/application/service/session_service.rs`, `src/adapter/inbound/tui/app/shell_runtime.rs`, `src/adapter/inbound/tui/app/shell_presentation/session_browser.rs`, `src/domain/recent_sessions.rs`, `src/domain/session_browser.rs`, `src/domain/session_summary.rs` | attach-only and handle-based reattach paths should not look broken just because provider-backed session listing is absent | the shell can truthfully render unsupported, partial, handle-based, or provider-backed session catalog states |
| 4 | approval and interrupt truth surface | `InteractiveTurnRuntime`, `TerminalBridgeAttachment` | `src/adapter/inbound/tui/conversation_text.rs`, `src/adapter/inbound/tui/app/conversation_runtime.rs`, `src/adapter/inbound/tui/app/conversation_model/view_model.rs`, `src/adapter/outbound/app_server/mod.rs` | current approval wording is inherited from app-server protocol limits, and interrupt support is implied rather than modeled as a capability truth | the shell can state runtime-native, manual-handoff, or unsupported behavior for approval and interrupt without pretending all bridges behave like Codex |
| 5 | attachment mode and recovery anchor seam | `TerminalBridgeAttachment`, `StartupProbe` | `src/adapter/outbound/app_server/runtime.rs`, `src/adapter/outbound/app_server/mod.rs`, plus new bridge-facing types under `src/application` or `src/domain` as needed | transport and recovery work should only begin after the earlier seams stop assuming that launch, reconnect, and resume are one app-server story | the runtime exposes launch vs attach mode, recovery anchor, and control limits explicitly enough for tmux-oriented attach or managed wrapper work to plug in later |

## Follow-Up Notes By Slice

### 1. Shell Capability Wording And Projection Split

Operator payoff:

- startup, session, and live-tail surfaces stop implying every supported path is a Codex app-server
- future bridge work gets one place to update wording instead of scattered string edits

Guardrails:

- do not rename every current Rust type just to match future wording
- keep the first slice focused on copy, projection helpers, and shell-facing status assembly

### 2. Outbound Capability Ownership Split

Operator payoff:

- startup, sessions, and turns become easier to reason about separately
- non-Codex work can satisfy only the capabilities it really has

Guardrails:

- current Codex adapter may continue to implement all split ports in one adapter
- avoid a monolithic `ProviderPort` replacement

### 3. Optional Session Catalog And Session-Tier Surface

Operator payoff:

- attach-only paths can still feel deliberate rather than degraded or broken
- reattach expectations become explicit instead of silently inherited from provider-backed sessions

Guardrails:

- do not make session listing a startup prerequisite
- keep session tier language aligned with
  [23-terminal-agent-capability-boundary-and-session-contract.md](23-terminal-agent-capability-boundary-and-session-contract.md)

### 4. Approval And Interrupt Truth Surface

Operator payoff:

- the shell can explain what the current path really supports before the operator gets stuck
- bridge experiments can report manual-handoff or unsupported behavior honestly

Guardrails:

- do not promise interactive approve or deny actions before a real path supports them
- keep approval and interrupt truth separate from transport branding

### 5. Attachment Mode And Recovery Anchor Seam

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
- update the capability map only when the current-state coupling materially changes

## Related Docs

- [17-structure-and-architecture-debt-map.md](17-structure-and-architecture-debt-map.md)
- [20-context-first-architecture-and-doc-coherence.md](20-context-first-architecture-and-doc-coherence.md)
- [21-terminal-agent-bridge-research-and-capability-boundary.md](21-terminal-agent-bridge-research-and-capability-boundary.md)
- [23-terminal-agent-capability-boundary-and-session-contract.md](23-terminal-agent-capability-boundary-and-session-contract.md)
- [25-codex-assumption-to-capability-target-map.md](25-codex-assumption-to-capability-target-map.md)
