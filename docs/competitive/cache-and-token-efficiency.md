# Cache and Token Efficiency

Status: focused competitive study

Audit date: 2026-08-08 (Asia/Seoul). Second snapshot and source read: 2026-11-13, see
[Second Snapshot](#second-snapshot-2026-11-13) and
[Source Read at the Installed Pin](#source-read-at-the-installed-pin-2026-11-13).

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

## Second Snapshot (2026-11-13)

One OmO Desktop session on Windows (Senpi
[`2638d524`](https://github.com/code-yeongyu/senpi/tree/2638d524bab2bb87cff874ef1f7e11df16c77643),
OmO [`v5.1.46`](https://github.com/code-yeongyu/oh-my-openagent/tree/638d30d68837e994ea2edb3a87cb94f40588d991),
both read from the bundle manifest). The session lasted about 52 minutes, delivered two Akra
Desktop pull requests, and reviewed two kernel commits. All models ran on the ChatGPT subscription
path.

| Role | Model | Requests | Uncached input | Cache read | Output | Cache-read fraction | Reported cost |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Orchestrator, last 12.5 minutes only | `gpt-6-astra` | 37 | 46,412 | 8,836,096 | 9,158 | **99.5%** | $9.76 |
| Six child tasks, complete | `gpt-6.1-sol` | 109 turns | 480,530 | 5,808,256 | 45,685 | **92.4%** | $2.00 |

Per child task: 6 to 30 turns, 13 to 91 tool calls, 72 to 387 seconds, $0.12 to $0.52. Two
re-reviews that were limited to one blocker cost $0.12 and $0.19. The first model call of a child
was 13k to 22k tokens for a prompt under 1k tokens, and it already had 3,968 cache-read tokens.

Reading the two rows together:

- The orchestrator re-read about 240k tokens on each response. In the same 12.5 minutes it cost
  about ten times as much as the children that did the reading and the reviews.
- The child tasks were cheap because their context was small and their scope was narrow, not
  because of the cache.
- A 99.5% cache-read fraction and a large total are the same design choice seen from two sides: the
  history is never reduced, so almost all of it is a cache hit, and all of it is paid for again on
  each response at the cache-read price.

Method: sum the numeric `usage` fields of assistant messages from the orchestrator's provider event
log, deduplicated by response id, and read `run_stats` from the six child task records. The audit
did not copy prompt text, response text, credentials, or session identifiers.

Limitations:

- the orchestrator's event log rotates at about 100 MB and kept only the last 12.5 minutes; the
  first 40 minutes are unknown;
- "reported cost" is the list price that the client computes; a subscription is not billed per
  token;
- one session, one workload, no control run, and no Akra run of the same work;
- the context window that the installed app used for `gpt-6-astra` was not read from the app. The
  Senpi catalog at the pin says 600,000.

## Source Read at the Installed Pin (2026-11-13)

Both repositories were read at the commits in the bundle manifest. The installed runtime is
bytecode, so the read is of the public source at the same commits, not of the binary. Line
references are to those commits.

### What explains the cache-read fraction

In order of weight:

1. **Append-only history with changes delivered as trailing messages.** A change of system prompt
   text or of the tool set is appended as a later system message, not written into the first
   message (`packages/ai/src/types.ts`, `packages/agent/src/agent-loop.ts`). For models that accept
   an allowed-tools list, every tool that was ever active stays declared, and narrowing is done
   with `tool_choice` (`coding-agent/src/core/agent-session.ts`, with a comment that names prefix
   preservation).
2. **A cache key made from the reusable prefix.** `prompt_cache_key` is a hash of the model id, the
   first system message, and the tools sorted by name with their descriptions and parameter
   schemas. The source comment: "Cache affinity belongs to the reusable leading prefix, never to a
   conversation UUID" (`packages/ai/src/api/openai-prompt-cache.ts`). Sessions with the same model,
   system prompt, and tools share one key. That is the probable source of the cache read on the
   first call of a child. A child does not share the orchestrator's prefix: it has the default
   system prompt and a filtered tool list.
3. **Volatile values kept out of the system prompt.** The working directory and the date go in a
   hidden `environment-context` message that is appended only when a value changes. The file
   comment says a date or cwd line in the system prompt "rewrites the provider-visible prefix every
   day and in every directory" (`coding-agent/src/core/environment-context.ts`). OmO does the same
   for memory: the memory block is pinned to the revision of the first turn, and later changes go
   in a trailing `<memory_notice>` message until the next compaction
   (`omo-senpi/src/components/memory/projection-pin.ts`).
4. **A 30-minute retention for current OpenAI models.**
   `PROMPT_CACHE_TTL_OPENAI_EXTENDED_SECONDS = 1800` for GPT-5.6 and later
   (`packages/ai/src/utils/prompt-cache-ttl.ts`). The 300-second constant in `cache-stats.ts` is
   only used for the wording of a miss notice. This corrects the 300/3,600-second reading in
   [Why Senpi Hits Cache](#why-senpi-hits-cache).
5. **A per-session WebSocket that sends only the new input** with `previous_response_id`, and a
   full replay when the prefix differs (`packages/ai/src/api/openai-codex-responses.ts`). This
   reduces bytes on the wire. Its effect on the cache-read fraction is indirect.

Not a cause in this run:

- **Keep-alive requests** are off by default, are limited to 3 requests and $0.05 for each session,
  and exist only for the direct Anthropic path (`settings-manager.ts`,
  `builtin/cache-keepalive/`).
- **Prewarm** fires once for each session on the first draft edit and only for explicit-cache
  models on `api.openai.com`. It does not apply to the subscription path.
- **The cache-safe wait budget** (retention minus 30 seconds) is exported to tools as a hint. The
  host does not enforce it.

### What controls total tokens

| Mechanism | Where | Effect |
| --- | --- | --- |
| Tool output limit of 2,000 lines or 50 KB, with the rest written to a file and a pointer returned | `coding-agent/src/core/tools/truncate.ts`, `bash.ts` | reduces tokens |
| `eval` code cells: when `eval` is registered, the shell, `grep`, and `glob` are callable only inside a cell; intermediate results stay in the kernel and only printed output returns (head 20 KB plus tail 50 KB) | `packages/senpi-codemode` | fewer round trips; the return limit is larger than the direct shell limit |
| Deferred tools behind a local `tool_search` (BM25, at most 5 results, schema returned on demand) | `builtin/tool-search/` | unused schemas are not in the request |
| Commands that pass a 60-second foreground window move to a background session; completion wakes the model with a bounded notice | `builtin/terminal/` | no model turns spent on waiting |
| Child tasks get no parent transcript; the parent gets only the final response, at most 32,000 characters, with the rest in a file | `senpi-task/src/completion/notification.ts` | bounded handoff in both directions |
| Tool-result admission limit of 5% of the window (8,192 to 50,000), and reduction of old tool results without a model call | `builtin/compaction/tool-admission.ts`, `context-reduction.ts` | reduces tokens, but **off for models that use OpenAI remote compaction** |
| Compaction at 70% of the window (80% above 512k), checked at the start of a prompt and when the agent goes idle; a hard limit of window minus reserve is checked during a run | `builtin/compaction/policy.ts` | with a 600k window, nothing fires before 480k |

The last two rows explain the orchestrator. The catalog gives `gpt-6-astra` a 600,000-token
window, so 240k is 40% of the window and under every trigger, and the no-model reduction of old
tool results does not run for that model family. The run did not break a policy. The policy allows
a long session to carry and re-read a quarter of a million tokens on each response.

Always-on context that raises the total: the OmO `ultrawork` directive (about 31k characters, once
for each session and again after each compaction), the memory block (up to 24 KB in the system
prompt), and the default system prompt plus tool definitions (12k to 21k tokens measured on a
child's first call).

### Corrections to this document

- The cache-read fraction in the first snapshot (75%) is not a ceiling. A long single session
  reached 99.5%.
- Senpi's retention for current OpenAI models is 30 minutes in code, not 5 minutes.
- "Goal cache-warm accounting" is a timer (270 seconds by default), not a request. No request is
  sent to keep a cache warm on the subscription path.

### What this adds to the Akra decisions

Nothing in the read changes the Adopt and Reject lists below. It sharpens three points:

1. A high cache-read fraction is a property of an append-only history. It must not be a target on
   its own; the same run shows that it can coexist with the highest cost in the session.
2. The parts worth copying are on the Akra side of the official boundary: keep volatile values at
   the end of what Akra sends, keep the instructions of a thread stable for its life, and bound
   what a child returns. Akra has not yet checked where its planning fragment and its
   thread-start instructions put volatile values. That check is proposed work.
3. Measurement comes first. Akra still does not project its own cache-read fraction, so it cannot
   say whether it has a problem.

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
