# Senpi + OmO Native

## Naming and Snapshot

The installed runtime is **Senpi** (`@code-yeongyu/senpi`), not “Senpai.” The installed “OmO
Native” surface is the `@code-yeongyu/omo-senpi` adapter running inside Senpi. It is not a third
independent application.

| Field | Value |
| --- | --- |
| Senpi repository | <https://github.com/code-yeongyu/senpi> |
| Installed Senpi | v2026.7.26, [`539c8a5c30dd1599d790052eed6609dfa0572c4f`](https://github.com/code-yeongyu/senpi/tree/539c8a5c30dd1599d790052eed6609dfa0572c4f) |
| Current Senpi release | [v2026.8.7](https://github.com/code-yeongyu/senpi/releases/tag/v2026.8.7), `00ad6a70a06b6331e8087912612726613d365491` |
| OmO repository | <https://github.com/code-yeongyu/oh-my-openagent> |
| Current public OmO release | [v4.19.4](https://github.com/code-yeongyu/oh-my-openagent/releases/tag/v4.19.4), `b072d279110bdda2c6ac2525d0d24dc54d16148a` |
| Installed OmO source | public `dev` commit [`fd79bf49bd45e2053658c7d52c512d7330037e34`](https://github.com/code-yeongyu/oh-my-openagent/tree/fd79bf49bd45e2053658c7d52c512d7330037e34); adapter package v4.19.4 |
| Audit environment | installed Windows packages plus WSL source/aggregate inspection |
| Audit date | 2026-08-08 (Asia/Seoul) |
| Akra baseline | `0c94f7e8b4549c2358f8b6d4b0c1ebc7e206dd48` |

The OmO v4.19.4 release calls itself the last release before the public Native CLI. Therefore
Native behavior at the installed dev commit is `verified/source` and `verified/runtime`, but not
a stable released contract. No settings secrets, prompts, responses, session IDs, or private
project names were retained.

## Verdict

Senpi is the strongest focused competitor for cache-aware, long-running terminal agent operation.
It owns provider requests and exposes cache reads, writes, cost, context, compaction, throughput,
session continuation, and cache-miss diagnostics. OmO Native adds skills, agent routing,
in-process/process child sessions, durable teams, and notification-driven persistence.

The combination is compelling because cache and orchestration are observable rather than hidden.
It does not demonstrate magic token reduction: a 75.07% local cache-read fraction still leaves all
cached tokens in context, and child sessions can increase total model work.

Akra should adopt the observability and bounded handoff discipline. It should not become another
provider harness or copy private cache-affinity behavior that official `codex app-server` owns.

## Product Boundary

```text
Senpi
  provider/auth/request transport
  session tree + compaction + usage accounting
  TUI + tools + extensions
        |
        +-- @code-yeongyu/omo-senpi
              skills and ultrawork directives
              model/category routing
              task + team tools
              child-session lifecycle and run statistics
```

Senpi controls prompt composition, provider calls, cache affinity, WebSocket continuation, model
fallback, compaction, and session persistence. This is a fundamentally wider boundary than Akra,
which consumes official app-server events.

The installed OmO adapter is packaged as one Senpi/Pi package with a generated extension and skill
set. Current public dev source documents:

- default in-process children using the parent's live tool closures except task/team recursion;
- process-mode children with persistent transcripts and respawn/resume;
- process-mode team members and durable inbox/processed ledgers;
- completion buffering across parent compaction, switching, and shutdown;
- background completion injection instead of model-visible polling;
- bounded transcript reads;
- last-request and whole-run cache-read fractions, output, total tokens, cost, and generation time.

## Cache and Token Findings

The sanitized installed snapshot contains 1,979 assistant requests and a 75.07% normalized
cache-read fraction. The full numerator, denominator, limitations, current Senpi source paths, and
Akra normalization are in [Cache and Token Efficiency](../cache-and-token-efficiency.md).

Important distinctions:

- installed Senpi v2026.7.26 has `prompt_cache_key`, `session-id`,
  `x-client-request-id`, WebSocket reuse, and `previous_response_id` continuation;
- current v2026.8.7 also applies `thread-id` affinity and adds newer cache regression/accounting
  paths;
- provider-reported `cacheWrite = 0` is not proof that no cache write occurred;
- a cache hit can reduce repeated-prefix latency or price but does not free context;
- OmO's OpenCode-only dynamic context-pruning settings must not be credited to Native without a
  Native implementation path;
- in-process children avoid process overhead and limit handoff context, but every child model call
  still consumes tokens.

## Current Competitive Shape

### Strengths

- Stable session-derived cache affinity and provider continuation.
- Fine-grained cache/context/cost/throughput instrumentation.
- Append-only session state, compaction events, resume, branching, and cache-miss notices.
- In-process child execution for low local overhead, with process mode when isolation/durability is
  needed.
- Notification-driven background work and durable team delivery.
- Explicit category/model routing and a large native skill surface.

### Limits Akra Can Exploit

- Senpi owns a large provider/auth/model/tool/compaction compatibility surface.
- The installed runtime is behind current Senpi while the OmO adapter is ahead of its public
  release; version skew complicates support and reproducibility.
- Public API cache retention and the private Codex subscription backend cannot be treated as the
  same contract.
- Delegation and adversarial multi-agent workflows can spend far more tokens than they save.
- The observed sessions do not establish isolated PR review, rebase merge, or cleanup invariants.
- Native release/distribution is still transitional at the audited OmO pin.

## Akra Decisions

### Adopt

- Last-request and cumulative cache-read fractions with explicit formulas and unavailable states.
- Cache-miss diagnostics correlated with model, idle, compaction, and reconnect facts.
- Notification-driven waiting and bounded child/tool result projection.
- Durable child identity, resume, and terminal delivery exactly once.
- Clear separation between in-process low-overhead workers and isolated delivery worktrees.

### Reject

- Provider request, auth, cache-key, affinity-header, or TTL ownership.
- Automatic model fallback that obscures the official Codex model actually serving a thread.
- Blind fanout, warm-up turns, or polling loops presented as token efficiency.
- Copying OpenCode-edition pruning claims into the Native comparison.

### Differentiate

- Keep Codex as the single official runtime authority.
- Tie parallel work to planning leases, exact-source worktrees, review, merge, and cleanup.
- Show cache/context efficiency beside delivery evidence, not as a replacement for it.

## Evidence

- [Senpi v2026.8.7 release](https://github.com/code-yeongyu/senpi/releases/tag/v2026.8.7)
- [Installed Senpi source pin](https://github.com/code-yeongyu/senpi/tree/539c8a5c30dd1599d790052eed6609dfa0572c4f)
- [OmO v4.19.4 release](https://github.com/code-yeongyu/oh-my-openagent/releases/tag/v4.19.4)
- [Installed OmO public-source pin](https://github.com/code-yeongyu/oh-my-openagent/tree/fd79bf49bd45e2053658c7d52c512d7330037e34)
- [OmO Senpi package](https://github.com/code-yeongyu/oh-my-openagent/tree/fd79bf49bd45e2053658c7d52c512d7330037e34/packages/omo-senpi)
- [Senpi task engine](https://github.com/code-yeongyu/oh-my-openagent/tree/fd79bf49bd45e2053658c7d52c512d7330037e34/packages/senpi-task)

Watch the first stable OmO Native release, installed Senpi/adapter version convergence, and a
controlled cold/warm Akra comparison before setting product targets.
