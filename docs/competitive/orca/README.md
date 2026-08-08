# StablyAI Orca

## Snapshot

| Field | Value |
| --- | --- |
| Official repository | <https://github.com/stablyai/orca> |
| Release | [v1.4.176](https://github.com/stablyai/orca/releases/tag/v1.4.176) |
| Source commit | [`02cea8a51ac3b69fcda7fe8ccc4f3ed0f68c445b`](https://github.com/stablyai/orca/tree/02cea8a51ac3b69fcda7fe8ccc4f3ed0f68c445b) |
| Release date | 2026-08-07 |
| Audit date | 2026-08-08 (Asia/Seoul) |
| Previous Akra pin | v1.4.137, `6013055491943336660e12e5dec93c9ece4575bb` |
| Akra baseline | `0c94f7e8b4549c2358f8b6d4b0c1ebc7e206dd48` |

Evidence is official release/source inspection. The previously identified Windows installation
established product identity; this refresh did not unpack and hash the latest binary.

## Verdict

Orca is the clearest product reference for a desktop fleet organized around repositories,
worktrees, terminals, agent sessions, source control, remote hosts, and mobile access. Its threat is
operational cohesion and recovery polish: users can see and reopen many parallel coding sessions
without thinking in raw process primitives.

Akra's edge remains narrower authority. Orca coordinates many external agent runtimes; Akra can
bind official Codex sessions to planning, exact source, reviewed integration, and cleanup.

## Material Change Since v1.4.137

The [compare range](https://github.com/stablyai/orca/compare/v1.4.137...v1.4.176) contains a very
high-churn interval. Current release history shows:

- terminal reattach/recovery, renderer-restart recovery, and detached-daemon ownership fencing;
- persisted worktree metadata, immediate SSH-worktree projection, and lazy/virtualized worktree
  views;
- native chat reconnect retention, mobile host controls, and bounded auto-connect behavior;
- resumed external-agent sessions reappearing in the fleet;
- per-worker model/effort overrides and parent grouping for subagent transcripts;
- an experimental agent-map view;
- bounded watcher, history, terminal, and reattach work plus explicit memory-profile accounting;
- continued browser, computer-control, source-control, permissions, and proxy recovery.

This strengthens Orca's core advantage: recovery and fleet state are product work, not incidental
plumbing.

## Current Competitive Shape

### Strengths

- Worktree-first repository and agent-session navigation.
- Integrated terminals, source control, files, browser/computer tools, and remote hosts.
- Deliberate reconciliation after renderer, terminal, daemon, network, or credential failure.
- Desktop/mobile continuity and increasingly dense performance/correctness work.

### Limits Akra Can Exploit

- External CLIs and terminal inference remain important authority boundaries.
- Broad desktop integration carries Electron, daemon, renderer, OS-permission, and remote-host
  complexity.
- Fleet visibility is not automatically review incorporation or protected-base delivery.
- Experimental orchestration and agent maps should not be scored as stable defaults.

## Akra Decisions

### Adopt

- Worktree/session fleet information design.
- Explicit reattach state, stale detection, ownership fencing, and recovery receipts.
- Persisted metadata that makes SSH/remote work visible before slow probes finish.
- Performance budgets for repeated watcher, history, terminal, and rendering work.

### Reject

- Terminal-title or output inference when app-server events exist.
- Desktop/OS integration breadth as the initial product goal.
- Calling an agent map authoritative without typed delivery state.

### Differentiate

- Codex-native session and approval semantics.
- Planning leases and exact-source isolation.
- Reviewed range, CI/review handling, merge, and cleanup as one durable state machine.

## Evidence

- [v1.4.176 release](https://github.com/stablyai/orca/releases/tag/v1.4.176)
- [Pinned source](https://github.com/stablyai/orca/tree/02cea8a51ac3b69fcda7fe8ccc4f3ed0f68c445b)
- [v1.4.137 to v1.4.176 comparison](https://github.com/stablyai/orca/compare/v1.4.137...v1.4.176)

Watch whether orchestration becomes a stable reviewed-delivery product and whether remote/mobile
control shares one typed authority rather than terminal-derived state.
