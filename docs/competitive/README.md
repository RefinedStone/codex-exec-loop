# Competitive Research

[한국어 요약](../ko/competitive/README.md)

Status: current snapshot, not implementation truth

Audit date: 2026-08-08 (Asia/Seoul)

Akra baseline: `0c94f7e8b4549c2358f8b6d4b0c1ebc7e206dd48` (`prerelease`)

This directory keeps one compact, current brief per product. It exists to decide what Akra should
adopt, reject, or differentiate—not to mirror vendor documentation or maintain a permanent feature
ledger. Superseded snapshots and detailed one-off reports remain available in Git history.

## Product Position

> Akra is the Codex-first operating and delivery layer that turns official `codex app-server`
> sessions into durable, inspectable, review-delivered work.

The position has three boundaries:

1. **Protocol-native:** follow official app-server semantics instead of owning a provider harness.
2. **Delivery-native:** keep planning authority, isolated worktrees, review evidence, integration,
   and cleanup as one lifecycle.
3. **One application truth:** project the same state through TUI, Admin, CLI, Telegram, and
   automation.

Provider count, tool count, and agent count are not product goals by themselves.

## Current Coverage

| Product | Current pin | Previous audited pin | Why it matters to Akra |
| --- | --- | --- | --- |
| [OpenAI Codex](upstream-codex/README.md) | [v0.147.0](https://github.com/openai/codex/releases/tag/rust-v0.147.0), `be6e8eac029b183056b7e4402879f15d2c85f61b` | v0.144.1 | upstream runtime and protocol authority |
| [Senpi + OmO Native](omo-native/README.md) | Senpi [v2026.8.7](https://github.com/code-yeongyu/senpi/releases/tag/v2026.8.7); OmO public v4.19.4 plus installed public-source dev adapter | new | cache affinity, context accounting, durable child sessions, token observability |
| [jcode](jcode/README.md) | [v0.68.0](https://github.com/1jehuang/jcode/releases/tag/v0.68.0), `fcf53909f8fe3b8cb1167dc3a03b178b0a635f39` | v0.43.0 | native multi-session harness, desktop, performance instrumentation |
| [OpenCode](opencode/README.md) | [v1.18.15](https://github.com/anomalyco/opencode/releases/tag/v1.18.15), `d7b115f623760e68a4749d16508a9eca350f246f` | v1.17.18 | broad TUI/server/desktop harness and session continuity |
| [Orca](orca/README.md) | [v1.4.176](https://github.com/stablyai/orca/releases/tag/v1.4.176), `02cea8a51ac3b69fcda7fe8ccc4f3ed0f68c445b` | v1.4.137 | worktree-first desktop fleet, terminal recovery, remote/mobile reach |
| [Agent Canvas](agent-canvas/README.md) | [v1.6.1](https://github.com/OpenHands/agent-canvas/releases/tag/v1.6.1), `43f091baf135142ed6c146f888f44a957141193f` | v1.2.1 | browser/desktop inspector, remote backends, durable automations |
| [Grok Build](grok-build/README.md) | public HEAD `afbc0fb710320c7add294c2106d447ecc3e3af2e` | `b189869b...` | full-stack Rust harness, TUI breadth, fast worktrees |

The focused cross-product study is [Cache and Token Efficiency](cache-and-token-efficiency.md).
Evidence grades, refresh rules, privacy boundaries, and snapshot requirements are in
[Methodology](methodology.md).

## Current Decisions

### Adopt

- Preserve official thread identity and long-lived app-server continuity wherever correctness
  allows it.
- Promote provider-reported cached-input facts, context pressure, compaction, model changes, and
  runtime restarts into truthful operator telemetry.
- Keep large tool results, child results, and planning handoffs bounded at their application
  boundaries.
- Match the best competitors on reconnect, resume, terminal recovery, narrow-layout readability,
  and measurable latency.
- Treat worktree creation performance as an adapter optimization only after measurement.

### Reject

- Reimplementing provider requests, authentication, cache keys, private affinity headers, or model
  fallback inside Akra.
- Treating a high cache-hit percentage as proof of lower context use, lower spend, or fewer total
  tokens.
- Blind cache-warming turns, hard-coded provider TTLs, or background polling that repeatedly
  replays model context.
- Broad agent/tool marketplaces as a substitute for a reliable reviewed-delivery lifecycle.
- Maintaining parallel English/Korean copies of fast-moving source audits.

### Differentiate

- Make reviewed delivery evidence, explicit high-risk bypass provenance, and cleanup visible as
  first-class state.
- Measure the whole lifecycle: session continuity, uncached input, compaction, output, worktree
  isolation, checks, review, merge, and cleanup.
- Reuse official Codex capabilities quickly while keeping planning and delivery authority outside
  the model harness.

## Directory Contract

Each product directory contains one `README.md` with:

- an immutable release or source pin;
- material change since the prior Akra audit;
- current strengths and limits;
- explicit Akra decisions;
- direct primary-source links and known evidence gaps.

Reusable app-server probes and terminal captures remain under
[upstream-codex](upstream-codex/README.md) because repository tests consume some of them. Generated
reports, screenshots created for completed UI slices, raw guard output, and translated copies do
not belong here.

## Maintenance

- Refresh a brief when its decision changes, its pin is older than 90 days, or a material release
  invalidates a claim.
- Replace the snapshot in place; use Git history for the old one.
- Prefer release notes, immutable source, installed package metadata, and reproducible runtime
  evidence over marketing copy.
- Do not turn a competitor finding directly into shipped Akra truth. Put accepted future work in
  an explicitly proposed plan or issue.
