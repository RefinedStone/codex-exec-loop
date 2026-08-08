# OpenAI Codex

## Snapshot

| Field | Value |
| --- | --- |
| Official repository | <https://github.com/openai/codex> |
| Release | [`rust-v0.147.0`](https://github.com/openai/codex/releases/tag/rust-v0.147.0) |
| Source commit | [`be6e8eac029b183056b7e4402879f15d2c85f61b`](https://github.com/openai/codex/tree/be6e8eac029b183056b7e4402879f15d2c85f61b) |
| Release date | 2026-08-07 |
| Audit date | 2026-08-08 (Asia/Seoul) |
| Previous Akra pin | v0.144.1, `44918ea10c0f99151c6710411b4322c2f5c96bea` |
| Akra baseline | `0c94f7e8b4549c2358f8b6d4b0c1ebc7e206dd48` |

Evidence is official release/source documentation plus Akra's checked-in app-server probes. This
refresh did not rerun a full interactive upstream TUI benchmark.

## Verdict

Codex is Akra's upstream authority, not a harness Akra should out-fork. The material risk is release
velocity: the v0.144.1 audit is already obsolete around plugins, thread organization, transcript
pagination, MCP, permissions, compaction, and security. Akra wins only when it follows typed
app-server semantics quickly while adding durable planning and reviewed delivery outside the
harness.

## Material Change Since v0.144.1

The official [v0.147.0 notes](https://github.com/openai/codex/releases/tag/rust-v0.147.0) and
[compare range](https://github.com/openai/codex/compare/rust-v0.144.1...rust-v0.147.0) show a
high-churn interval. Decision-relevant changes include:

- portable Agent Plugins and local/personal/workspace/remote catalogs;
- persistent manually ordered conversation sections and incremental long-transcript browsing;
- `--approve-for-me` and broader permission-profile propagation;
- MCP 2026-07-28 pagination, multi-round requests, and non-blocking startup;
- remote compaction and cached web search on additional provider paths;
- secret redaction in commands and replayed history;
- fixes for focus/input loss, Unicode/cursor geometry, Windows process interruption, and paths;
- explicit project trust, managed-auth enforcement, and fail-closed plugin/network isolation;
- app-server stdio shutdown when its controlling connection closes.

The old finding that Akra should preserve host-terminal scrollback is no longer a current product
decision. Akra now owns an alternate-screen fullscreen TUI and should compare semantic transcript,
selection, resume, and viewport behavior instead.

## Current Competitive Shape

### Strengths

- Provider and tool authority live in one in-process Rust runtime.
- The TUI and app-server consume the same typed core events.
- SQLite-backed thread metadata, transcript pagination, plugins, MCP, skills, permissions, and
  multi-agent behavior evolve together.
- Upstream owns model selection, request caching, compaction, sandboxing, trust, authentication,
  and secret handling.
- Release and platform assurance are much broader than Akra can economically reproduce.

### Limits Akra Can Exploit

- Codex does not make Akra's planning queue, worktree leases, review evidence, protected-base
  integration, and cleanup lifecycle its primary product.
- Generic conversation organization is not the same as reviewed-delivery authority.
- Rapid protocol change creates room for a client that projects new capabilities clearly and
  preserves operational evidence across surfaces.

## Akra Decisions

### Adopt

- Regenerate and diff the official protocol on upgrades; preserve unknown-event observability.
- Follow thread pagination, sections, permissions, plugins, compaction, and token-usage events
  through application-owned projections.
- Match upstream correctness on input focus, Unicode width, Windows process lifecycle, trust, and
  secret redaction.
- Keep app-server connection ownership explicit and treat connection close as process lifecycle.

### Reject

- Provider request construction, model routing, cache keys, authentication, sandbox, or compaction
  reimplementation.
- Scraping upstream TUI text when typed app-server facts exist.
- Treating upstream release notes as proof that Akra already supports the corresponding protocol.

### Differentiate

- Durable queue and lease authority.
- Isolated parallel worktrees tied to exact source.
- Review/check/rebase/merge/cleanup evidence, including visible provenance for explicit bypasses.
- One operator model across TUI, Admin, CLI, Telegram, and automation.

## Evidence

- [Codex release](https://github.com/openai/codex/releases/tag/rust-v0.147.0)
- [Codex App Server documentation](https://developers.openai.com/codex/app-server)
- [Pinned source](https://github.com/openai/codex/tree/be6e8eac029b183056b7e4402879f15d2c85f61b)
- [v0.144.1 to v0.147.0 comparison](https://github.com/openai/codex/compare/rust-v0.144.1...rust-v0.147.0)

The local [scripts](scripts/) remain reusable probes. [Captures](captures/) are historical pinned
fixtures; their embedded v0.144.1 version must not be read as the current competitive snapshot.
