# Cache and Token Efficiency

Status: focused competitive study

Audit date: 2026-08-08 (Asia/Seoul)

Scope: OpenAI prompt caching, Senpi, OmO Native, and Akra's official app-server boundary

## Verdict

Senpi and OmO Native have unusually good cache observability and deliberate session affinity. A
sanitized snapshot of the installed Windows sessions showed a **75.07% provider-reported
cache-read fraction** across 1,979 assistant requests. That is strong evidence of sustained prefix
reuse in those sessions. It is not proof of a 75% reduction in context, tokens, spend, or latency.

Akra should copy the measurement discipline and continuity behavior, not the provider harness.
Official `codex app-server` already owns request construction and reports cached input. Akra's
highest-leverage work is to keep official threads stable, avoid duplicated context, bound results,
project the reported facts accurately, and let upstream own cache keys and compaction.

## Four Different Efficiency Problems

| Problem | Useful measure | Cache hits solve it? |
| --- | --- | --- |
| repeated-prefix latency and price | cached-input share plus measured latency/spend | partly |
| context-window pressure | current input/context window and compaction result | no |
| total model work | uncached input, cached input, output, reasoning, child fanout | no |
| local runtime overhead | processes, memory, I/O, reconnect/restart time | no |

Cached tokens still occupy model context. OpenAI also states that cached prompts count toward
rate limits and that output generation is unaffected. For current GPT-5.6+ API models, cache writes
and reads have different prices, so a hit percentage alone cannot calculate savings. See the
[official prompt-caching guide](https://developers.openai.com/api/docs/guides/prompt-caching).

## Normalized Metrics

Senpi's Pi usage model separates uncached input, cache reads, and cache writes:

```text
senpi_cache_read_fraction =
  cacheRead / (input + cacheRead + cacheWrite)
```

The Codex app-server schema reports `cachedInputTokens` as a subset of `inputTokens`. Akra
validates that relationship and treats `totalTokens = inputTokens + outputTokens`. The equivalent
projection is therefore:

```text
akra_cache_read_fraction =
  cachedInputTokens / inputTokens
```

Adding app-server `cachedInputTokens` to `inputTokens` would double count. A missing denominator
or missing provider telemetry must render as unavailable, not zero.

Neither formula is a financial savings formula. Provider-specific read/write prices, subscription
semantics, service tier, model, and unreported writes still matter.

## Installed Windows Snapshot

The installed product is **Senpi**, not “Senpai.” “OmO Native” is the Senpi adapter
`@code-yeongyu/omo-senpi`, not a third independent runtime.

| Field | Observed value |
| --- | ---: |
| Snapshot | one consistent rolling read on 2026-08-08 |
| Session JSONL files | 6 |
| Assistant requests | 1,979 |
| Requests with nonzero cache reads | 1,960 |
| Uncached input | 67,783,167 |
| Cache read | 204,079,872 |
| Cache write | 0 |
| Output | 607,973 |
| Cache-read fraction | **75.07%** |
| Provider-labelled requests | 1,977 `openai-codex`; 2 `claude-sdk-oauth` |

Method: stream the session JSONL files below the user's Senpi data directory, select assistant
`message` records, and sum only the numeric `message.usage` fields. The audit did not read or
export prompt text, response text, credentials, account IDs, session IDs, or private repository
names.

Limitations:

- the files were live and may change immediately after the snapshot;
- six sessions with different workloads are pooled;
- `cacheWrite = 0` can mean that the provider path does not report writes;
- there is no cold-cache control group, Akra comparison, latency trace, or exact billing record;
- the Codex subscription backend is not necessarily governed by public API pricing or retention;
- the result is a local observation, not a release benchmark or Akra target.

## Why Senpi Hits Cache

The installed Senpi package is v2026.7.26
([`539c8a5c...`](https://github.com/code-yeongyu/senpi/tree/539c8a5c30dd1599d790052eed6609dfa0572c4f)).
That version already:

- sends a stable session-derived `prompt_cache_key`;
- sends `session-id` and `x-client-request-id` affinity headers;
- reuses a per-session WebSocket;
- uses `previous_response_id` for a compatible continuation and falls back to full replay when
  the anchor is stale;
- records input, cache-read, cache-write, output, cost, and significant cache misses.

Current Senpi v2026.8.7
([`00ad6a70...`](https://github.com/code-yeongyu/senpi/tree/00ad6a70a06b6331e8087912612726613d365491))
adds a shared affinity helper that also sends `thread-id`, cache-affinity regression tests, goal
cache-warm accounting, and an extension-facing safe foreground-wait budget. The release itself
deliberately moved long-running Bash commands to background sessions instead of holding the model
turn open for that budget.

Relevant current source:

- [Codex Responses transport](https://github.com/code-yeongyu/senpi/blob/00ad6a70a06b6331e8087912612726613d365491/packages/ai/src/api/openai-codex-responses.ts)
- [cache-affinity helper](https://github.com/code-yeongyu/senpi/blob/00ad6a70a06b6331e8087912612726613d365491/packages/ai/src/api/openai-prompt-cache.ts)
- [cache-miss accounting](https://github.com/code-yeongyu/senpi/blob/00ad6a70a06b6331e8087912612726613d365491/packages/coding-agent/src/core/cache-stats.ts)
- [prompt-cache TTL abstraction](https://github.com/code-yeongyu/senpi/blob/00ad6a70a06b6331e8087912612726613d365491/packages/ai/src/utils/prompt-cache-ttl.ts)

Senpi's generic TTL abstraction currently uses 300 seconds for its short cache and 3,600 seconds
for its long cache. The official GPT-5.6+ API guide now documents a 30-minute default/minimum
retention and different implicit/explicit breakpoint behavior. That does not prove Senpi's private
Codex backend is wrong, but it does prove Akra must not copy Senpi's TTL constants as provider
truth.

## What OmO Native Adds

The installed public-source checkout is on the OmO `dev` branch at
[`fd79bf49...`](https://github.com/code-yeongyu/oh-my-openagent/tree/fd79bf49bd45e2053658c7d52c512d7330037e34);
its Senpi adapter package reports v4.19.4. The latest public release,
[v4.19.4](https://github.com/code-yeongyu/oh-my-openagent/releases/tag/v4.19.4), explicitly
precedes the public OmO Native CLI, so Native behavior is classified as public-source/local
evidence rather than a stable release contract.

The Native adapter contributes:

- in-process child sessions by default, avoiding an extra process while withholding task/team tools
  from children;
- resumable process-mode children and crash reconciliation;
- bounded transcript reads and batched completion injection;
- notification-driven background work instead of repeated model-visible polling;
- per-run cache-read fractions, provider spend when reported, output, total tokens, and generation
  throughput.

These features can reduce duplicated handoff context and local overhead. Delegation still creates
new model sessions and can increase total tokens. The dynamic tool-output pruning options documented
for OmO's OpenCode edition are not evidence that OmO Native enables the same pruning path.

## Akra's Current Boundary

Akra already parses `thread/tokenUsage/updated` into last and cumulative input, cached input,
output, reasoning output, total tokens, and context window:

- [protocol parser](../../src/adapter/outbound/app_server/protocol/progressive_activity.rs)
- [domain validation](../../src/domain/conversation_progressive_activity.rs)
- [protocol schema](../../schema/codex_app_server_protocol.v2.schemas.json)

The TUI exposes raw cached-input facts in the expandable token-usage card and shows context pressure
in the activity rail/operator ribbon. It does not yet promote last/run cache-read fractions,
cache-miss causes, or app-server restart correlation into stable operator metrics. App-server does
not expose cache-write price facts through this notification.

## Akra Decisions

### Adopt

1. Project last and cumulative `cachedInputTokens / inputTokens` when the denominator exists.
2. Correlate cache drops with observed thread restart, app-server reconnect, model change,
   compaction, and idle duration without claiming causation.
3. Keep one official thread and app-server process alive across normal TUI navigation and resume.
4. Continue bounding tool cards, child results, planning handoffs, and retained UI history.
5. Measure cold/warm latency and token facts together before setting any target.

### Reject

- direct control of `prompt_cache_key`, affinity headers, API breakpoints, or cache retention;
- synthetic turns whose only purpose is warming a cache without controlled net-benefit evidence;
- hard-coded provider TTLs;
- a “tokens saved” counter derived only from cached-token volume;
- copying prompts, responses, or identifiers into telemetry.

### Priority Order

For extreme token efficiency, optimize in this order:

1. remove duplicate static instructions and stale documentation;
2. preserve official session continuity;
3. prevent unnecessary full replays and polling turns;
4. bound tool and child outputs;
5. use upstream compaction based on real context pressure;
6. constrain delegation to work that repays its extra context;
7. optimize cache warming only after an A/B trace proves a net benefit.

This keeps cache efficiency subordinate to end-to-end delivery efficiency.
