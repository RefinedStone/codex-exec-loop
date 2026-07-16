# Grok Build Analysis

## Snapshot

| Field | Value |
| --- | --- |
| Product | Grok Build (`grok` / `xai-grok-pager`) |
| Official repository/site | https://github.com/xai-org/grok-build · https://x.ai/cli |
| Release/tag | Open-source publish commit (no public semver tag in shallow clone) |
| Source commit | `b189869b7755d2b482969acf6c92da3ecfeffd36` |
| Release date | 2026-07-15 |
| Audit date | 2026-07-16 (Asia/Seoul) |
| Akra baseline | `12bad93d08232a7626786b5edad91f52c2f5a6fc` (`prerelease`) |
| Auditor environment | macOS, static source audit of shallow clone at `/tmp/grok-build` |

Checkout is public, Apache-2.0 for first-party code. External contributions are not accepted.
Interactive HTML report: [../reports/grok-build/index.html](../reports/grok-build/index.html).

## Evidence Discipline

Findings use the parent [evidence rules](../README.md). Ledger: [evidence.md](./evidence.md). Decisions: [gap-matrix.md](./gap-matrix.md).

## Executive Verdict

Grok Build is SpaceXAI’s terminal coding agent: full-screen TUI, in-process agent runtime,
tool harness, sandbox, MCP/skills/plugins/hooks, subagents, plan mode, ACP, headless, and
leader multi-client continuity. It is a **provider-owned harness**, not a protocol client over
another app-server.

For Akra the threat is **feature breadth and TUI/extension polish**, not delivery authority.
Akra’s structural edge remains SQLite planning, worktree-isolated parallel delivery with review
gates, and multi-surface operator control (TUI/Admin/Telegram).

**Adopt** plan-approval UX, multi-session dashboard information design, host-side policy hooks,
PTY e2e density, and (after measurement) fast-worktree ideas. **Reject** owning a model/tool
harness, marketplace/provider breadth, OS sandbox reimplementation, and yolo/session-wide grants
as defaults. **Differentiate** harder on review-delivered parallel pool and durable queue authority.

## Product And Audience

- Primary operator: individual/team developer in terminal or IDE via ACP
- Surfaces: TUI, headless CLI, ACP stdio/WebSocket, agent dashboard
- Distribution: install scripts for macOS/Linux/Windows; self-update; Rust source build
- Auth: browser OAuth, API key, OIDC SSO, device code
- Provider assumption: SpaceXAI Grok models by default; custom models documented

## Architecture

- Runtime authority lives in `xai-grok-shell` + tools/workspace crates (in-process)
- Leader process shares agent state across TUI/IDE/headless over Unix socket
- Persistence under `~/.grok/` (auth, config, sessions, memory, trust)
- Module split: pager (TUI) / shell (agent) / tools / workspace / fast-worktree / MCP / hooks / sandbox
- Recovery: session resume/fork, durable log reattach tests, folder trust, config hot-reload paths

## Harness And Context

- Built-in tool zoo + codex/opencode-ported implementations under taxonomy
- MCP stdio/HTTP/SSE; skills; plugins + marketplace; hooks with Claude/Cursor compat
- Auto-compact threshold; experimental cross-session memory (FTS5 + optional vectors)
- Permission modes, allow/deny rules, PreToolUse blocking hooks, OS sandbox profiles

## TUI And UX

- Full-screen scrollback + prompt; minimal mode; vim mode; theming; mermaid
- Plan mode approval with line comments
- Dashboard for multi-agent session board
- Dense `pty_e2e` / `leader_pty_e2e` coverage for terminal edge cases

## Admin And Remote Surfaces

- No Akra-style game Admin control center
- ACP + leader attach is the remote/IDE plane
- External OTEL for org collectors (double opt-in, content-free default)

## Parallel Work

- Subagents with types (`general-purpose`, `explore`, `plan`) and personas
- Optional `isolation: worktree` + apply-back via workspace RPC
- Not a lease-serialized PR delivery control plane

## Performance

No comparable process-tree benchmarks in this audit. Fast-worktree claims are architectural only.

## Safety

Strong layered model: hooks → rules → grants → modes → sandbox. Remembered grants and
`bypassPermissions` optimize interactive speed; Akra should not copy those as unattended defaults.

## Strengths

1. Complete extension stack with competitor settings compatibility
2. Leader multi-client continuity and ACP productization
3. Plan-mode human gate UX
4. OS sandbox + permission vocabulary depth
5. PTY e2e culture
6. CoW/BTRFS worktree engineering

## Weaknesses (relative to Akra position)

1. Not review-delivered by default; PR/integration control plane is not the product center
2. Planning is session-file oriented, not durable multi-surface authority
3. Provider-coupled; open source is sync-from-monorepo, external contrib closed
4. Feature surface invites scope gravity that Akra should avoid

## Implications For Akra

Keep Codex-first delivery layer. Steal UX/validation patterns. Do not become another broad harness.
Full decision table: [gap-matrix.md](./gap-matrix.md).
