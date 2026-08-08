# OpenCode

## Snapshot

| Field | Value |
| --- | --- |
| Official repository | <https://github.com/anomalyco/opencode> |
| Release | [v1.18.15](https://github.com/anomalyco/opencode/releases/tag/v1.18.15) |
| Source commit | [`d7b115f623760e68a4749d16508a9eca350f246f`](https://github.com/anomalyco/opencode/tree/d7b115f623760e68a4749d16508a9eca350f246f) |
| Release date | 2026-08-07 |
| Audit date | 2026-08-08 (Asia/Seoul) |
| Previous Akra pin | v1.17.18, `b1fc8113948b518835c2a39ece49553cffe9b30c` |
| Akra baseline | `0c94f7e8b4549c2358f8b6d4b0c1ebc7e206dd48` |

Evidence is release and immutable source inspection. No current provider-cost or interactive
terminal benchmark was reproduced.

## Verdict

OpenCode remains the broadest direct reference for a unified TUI, server, web, desktop, SDK, ACP,
provider, plugin, permission, and automation ecosystem. Its threat is continuity across surfaces
and a large contributor/release loop, not one unique feature.

Akra should copy its session-correctness discipline and operator clarity while rejecting provider
and marketplace ownership. A general harness can optimize cache keys directly; Akra must instead
project the official app-server's cache and context facts.

## Material Change Since v1.17.18

The [compare range](https://github.com/anomalyco/opencode/compare/v1.17.18...v1.18.15) contains
roughly 480 commits. Material changes include:

- provider prompt-cache-key selection and serialization fixes;
- cache-write tokens included in ACP usage;
- current-session timelines, v1 progress hydration, and paginated timeline ordering;
- an opt-in v2 desktop sidecar and continued desktop composition work;
- fixes for stale session-tab and prompt-control reads;
- session ordering based on persisted chronology/activity rather than sortable IDs;
- repeated compaction preserving earlier tool-call history and orphaned compaction serialization;
- session JSON export and broader desktop localization;
- MCP session recovery and TUI cursor/tmux copy improvements.

The changes reinforce that cache accounting and session chronology are correctness contracts, not
decorative statistics.

## Current Competitive Shape

### Strengths

- One server model supports TUI, web, desktop, SDK, ACP, and attach/resume workflows.
- Provider/model breadth, plugins, permissions, and GitHub automation create a large ecosystem.
- Session timelines and persisted chronology make long-running work easier to inspect.
- Cache-read/write and compaction state receive first-class maintenance.

### Limits Akra Can Exploit

- Provider and compatibility breadth create a large security and regression surface.
- Server/session continuity does not establish Akra's reviewed-delivery invariants.
- A general harness cannot be the authoritative source for official Codex-only semantics.
- Direct cache controls are not portable to Akra's app-server boundary.

## Akra Decisions

### Adopt

- Chronology based on persisted timestamps and authoritative final events.
- Explicit progress/timeline recovery after reconnect or resume.
- Cache and compaction telemetry that distinguishes unavailable, zero, read, and write.
- Session export and bounded inspection without loading entire histories eagerly.

### Reject

- Provider SDK and prompt-cache-key ownership.
- Marketplace breadth as Akra's primary differentiation.
- Desktop v2 duplication before application projections are complete.

### Differentiate

- Official Codex protocol fidelity.
- Durable planning and worktree authority.
- Review/check/merge/cleanup evidence shared by every operator surface.

## Evidence

- [v1.18.15 release](https://github.com/anomalyco/opencode/releases/tag/v1.18.15)
- [Pinned source](https://github.com/anomalyco/opencode/tree/d7b115f623760e68a4749d16508a9eca350f246f)
- [v1.17.18 to v1.18.15 comparison](https://github.com/anomalyco/opencode/compare/v1.17.18...v1.18.15)

Watch the v2 desktop/server boundary, cache accounting across every client protocol, and whether its
delivery lifecycle grows protected-base review guarantees.
