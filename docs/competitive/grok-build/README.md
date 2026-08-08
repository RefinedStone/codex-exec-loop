# Grok Build

## Snapshot

| Field | Value |
| --- | --- |
| Official repository | <https://github.com/xai-org/grok-build> |
| Public release | no GitHub semver release |
| Public source commit | [`afbc0fb710320c7add294c2106d447ecc3e3af2e`](https://github.com/xai-org/grok-build/tree/afbc0fb710320c7add294c2106d447ecc3e3af2e) |
| Monorepo `SOURCE_REV` | `3e620a76a5f374ce644dc7c87f7e990c68348218` |
| Source date | 2026-08-07 |
| Audit date | 2026-08-08 (Asia/Seoul) |
| Previous Akra pin | `b189869b7755d2b482969acf6c92da3ecfeffd36` |
| Akra baseline | `0c94f7e8b4549c2358f8b6d4b0c1ebc7e206dd48` |

The previous public pin is not reachable through the current GitHub object/compare endpoints, so a
reliable old-to-new commit delta cannot be claimed. This brief is a fresh static audit of the
current synchronized source, not a release comparison or runtime benchmark.

## Verdict

Grok Build remains a broad provider-owned Rust harness: full TUI and minimal pager, agent runtime,
tools, MCP, skills, plugins/hooks, sandbox/policy, memory, plan mode, ACP, headless operation,
session continuity, and worktree support. Its threat is full-stack polish and the ability to change
the model/runtime/UI together.

Akra should use Grok as an architecture and terminal-quality reference, not copy the harness.
Akra's durable planning and reviewed-delivery authority remain the sharper position.

## Current Competitive Shape

Current source verifies:

- a large Rust workspace with separate pager, shell, tools, workspace, telemetry, update, voice,
  and fast-worktree crates;
- full and minimal terminal render paths with extensive PTY coverage;
- session fork/list/destroy/rewind and worktree-isolated session types;
- explicit permission requests and policy changes across workspace protocols;
- MCP, skills, hooks/plugins, memory search/write, plan/subagent flows, and ACP;
- `xai-fast-worktree` paths for reflink/copy, Btrfs, overlay, sync, removal, and integration tests;
- cached-prompt token fields in telemetry, without enough local evidence for a cache-efficiency
  comparison.

### Strengths

- In-process coordination across provider, tool runtime, permissions, TUI, and session state.
- Dense terminal and workspace test surface.
- Plan/dashboard UX and broad extension compatibility.
- Serious investment in worktree creation and synchronization performance.

### Limits Akra Can Exploit

- The public repository is a synchronized snapshot, making longitudinal public audit harder.
- Provider/runtime breadth carries substantial maintenance and security surface.
- Fast worktree creation optimizes isolation startup; it does not prove reviewed integration.
- Current public source does not establish Akra's planning queue, review handling, merge, and
  cleanup invariants.

## Akra Decisions

### Adopt

- PTY/terminal edge-case density and explicit full/minimal rendering invariants.
- Plan approval and multi-session information design.
- Host-side policy-hook ideas that do not replace official Codex permissions.
- A measured fast-worktree adapter experiment on repositories where checkout dominates startup.

### Reject

- Provider/tool/sandbox/marketplace ownership.
- Grok-specific private runtime or telemetry assumptions.
- Fast-worktree complexity before a versioned Akra benchmark shows a meaningful bottleneck.

### Differentiate

- Official app-server fidelity.
- Planning authority and durable leases.
- Review-delivered worktree integration with cleanup evidence.

## Evidence

- [Current public source](https://github.com/xai-org/grok-build/tree/afbc0fb710320c7add294c2106d447ecc3e3af2e)
- [Repository README](https://github.com/xai-org/grok-build/blob/afbc0fb710320c7add294c2106d447ecc3e3af2e/README.md)
- [Fast worktree crate](https://github.com/xai-org/grok-build/tree/afbc0fb710320c7add294c2106d447ecc3e3af2e/crates/codegen/xai-fast-worktree)
- [Workspace protocol types](https://github.com/xai-org/grok-build/tree/afbc0fb710320c7add294c2106d447ecc3e3af2e/crates/codegen/xai-grok-workspace-types)

Watch for a stable public release/tag and a reachable history before making quantitative change or
performance claims.
