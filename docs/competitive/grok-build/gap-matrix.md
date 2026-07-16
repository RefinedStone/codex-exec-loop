# Grok Build Gap Matrix (Akra-relative)

Akra baseline: `12bad93d` · Grok Build: `b189869b` · Audit: 2026-07-16

## Decision Legend

| Decision | Meaning |
| --- | --- |
| **Adopt** | Improves Akra’s Codex-first operator/delivery position; maps to owned boundary |
| **Eval** | Promising; needs proof target before product commitment |
| **Reject** | Would dilute position, duplicate app-server, or weaken safety |
| **Differentiate** | Akra already stronger or should invest further on this axis |

## Matrix

| Area | Grok Build signal | Akra today | Decision | Testable work |
| --- | --- | --- | --- | --- |
| Product identity | Provider-owned harness | Codex-first delivery layer | **Differentiate** | Keep position text in competitive README; no harness expansion PR |
| Plan approval UX | plan.md + line comments + a/s/c/q | draft→validate→promote | **Adopt** | Plan preview surface; comment provenance on promote receipt |
| Multi-session board | Dashboard needs-input/working/idle | sessions + parallel board | **Adopt** | Unified board projection; withheld-dispatch reason visible |
| Host policy hooks | PreToolUse + allow/deny | fail-closed approvals | **Adopt** | Hooks at planning mutation / delivery gates; audit log |
| PTY e2e density | Large pty_e2e suite | terminal captures + matrix | **Adopt** | Scenario IDs for resize/paste/flood/reattach |
| Hunk attribution | agent vs external tracker | changed-file bounds | **Eval** | Attribution metadata in cleanliness/lease checks |
| Fast worktree | CoW/BTRFS pool | git worktree pool | **Eval** | Latency bench on large repo; no policy change. Deep dive: [fast-worktree.md](./fast-worktree.md) / [fast-worktree.html](../reports/grok-build/fast-worktree.html) |
| Background monitor/loop | tool + /loop | review poller services | **Eval** | Operator-facing monitor only if maps to application service |
| Tool taxonomy meta | ToolKind envelope | progressive activity kinds | **Eval** | Projection vocabulary only |
| In-process tools/model | Full tool runtime | app-server tools | **Reject** | — |
| Marketplace/plugins product | plugins + market | planning skill boundary | **Reject** | — |
| OS sandbox engine | Seatbelt/bwrap profiles | app-server sandbox policy | **Reject** | — |
| YOLO / session grants | bypass + remembered grants | no session-wide grant cache | **Reject** | Guard tests remain fail-closed |
| Parallel delivery | subagent worktree optional | 3-slot lease + PR review | **Differentiate** | Deepen frozen delivery + review provenance |
| Planning authority | session plan file | SQLite queue authority | **Differentiate** | Recovery/distributor tests |
| Multi-surface ops | TUI/ACP/leader | TUI/CLI/Admin/Telegram | **Differentiate** | Shared application truth invariants |

## Priority Slice Order

1. **P0** Plan promote UX (preview + comments)
2. **P0** Sessions/parallel board information design
3. **P1** Host policy hooks for delivery/planning
4. **P1** PTY scenario suite densification
5. **P2** Dirty attribution for delivery bounds
6. **P2** Worktree create latency experiment

## Non-Goals

- Cloning Grok Build product surface
- Owning MCP marketplace or media generation tools
- Replacing codex app-server with in-process harness
