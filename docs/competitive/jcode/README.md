# jcode

## Snapshot

| Field | Value |
| --- | --- |
| Official repository | <https://github.com/1jehuang/jcode> |
| Release | [v0.68.0](https://github.com/1jehuang/jcode/releases/tag/v0.68.0) |
| Source commit | [`fcf53909f8fe3b8cb1167dc3a03b178b0a635f39`](https://github.com/1jehuang/jcode/tree/fcf53909f8fe3b8cb1167dc3a03b178b0a635f39) |
| Release date | 2026-08-05 |
| Audit date | 2026-08-08 (Asia/Seoul) |
| Previous Akra pin | v0.43.0, `649276753ae11948759192c067dfc4c90fafd47f` |
| Akra baseline | `0c94f7e8b4549c2358f8b6d4b0c1ebc7e206dd48` |

Evidence is immutable source and official release history. The published performance claims were
not rerun on matching hardware, so no speed winner is declared.

## Verdict

jcode remains the strongest warning that “native Rust wrapper with a TUI” is not a product
position. It owns a provider/runtime stack, shared multi-session services, memory, hooks, swarm,
terminal integration, telemetry, and a rapidly maturing native desktop. Its threat is the
combination of breadth, speed as a product value, and dense session instrumentation.

Its structural cost is the same breadth. Akra should not compete by owning providers, auth, memory,
desktop composition, swarm, and model compatibility. It should make the official Codex path more
operationally trustworthy from accepted intent through reviewed integration.

## Material Change Since v0.43.0

The [compare range](https://github.com/1jehuang/jcode/compare/v0.43.0...v0.68.0) contains roughly
1,300 commits. Material changes include:

- a much larger `desktop2` surface with rich Markdown, readable diffs, a model picker, background
  progress, settings, smoother transcript navigation, and per-session overview cards;
- resuming a stored session without leaving the current desktop session;
- a Rust SDK path between desktop and the harness;
- Windows terminal spawning, named-pipe bridge support, and Windows ARM64 release artifacts;
- provider-specific tool-schema dialects and recovery from schema incompatibilities;
- provider credential lifecycle and removal of implicit provider credentials from MCP servers;
- context-window resolution tests, bounded tool output, token-value/cost telemetry, and fallback
  routing fixes;
- swarm workers retaining the intended memory scope.

The v0.43.0 conclusion that desktop was mainly proposed is no longer current.

## Current Competitive Shape

### Strengths

- Native performance and resource use are treated as user-visible product properties.
- TUI and desktop expose model, context, git, todos, background work, memory, and swarm state.
- Shared services reduce repeated setup across sessions and make attach/resume central workflows.
- Provider dialect and fallback layers absorb heterogeneous model behavior.
- Cross-platform distribution now includes Linux, macOS, Windows, FreeBSD, and Windows ARM64.

### Limits Akra Can Exploit

- A large provider/auth/tool/memory/swarm/desktop surface creates security and maintenance cost.
- Shared multi-session execution does not by itself prove isolated worktree delivery, review
  incorporation, protected-base integration, or cleanup.
- Performance claims still need a controlled, versioned comparison before Akra should trade away
  correctness or architecture.

## Akra Decisions

### Adopt

- Measurable submit latency, render latency, resume latency, memory, and long-session behavior.
- Space-aware context, background-work, model, git, and task instrumentation.
- Resume/reconnect without abandoning the operator's current workspace.
- Bounded search/tool output and explicit credential lifecycle states.

### Reject

- Provider/model dialect ownership.
- A separate desktop runtime before TUI and Admin share complete application truth.
- Swarm or memory breadth without reviewed-delivery evidence.

### Differentiate

- Official Codex protocol fidelity.
- Worktree and planning authority tied to exact source.
- Review-delivered parallel work and cleanup receipts.
- Visible safety provenance when high-risk automation bypasses ordinary gates.

## Evidence

- [v0.68.0 release](https://github.com/1jehuang/jcode/releases/tag/v0.68.0)
- [Pinned source](https://github.com/1jehuang/jcode/tree/fcf53909f8fe3b8cb1167dc3a03b178b0a635f39)
- [v0.43.0 to v0.68.0 comparison](https://github.com/1jehuang/jcode/compare/v0.43.0...v0.68.0)

Watch the desktop/session SDK boundary, repeatable performance artifacts, and whether swarm work
acquires an explicit reviewed integration lifecycle.
