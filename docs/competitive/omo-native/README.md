# OmO: Harness, Codex Light Edition, and Desktop

## Naming and Snapshot

OmO now ships as three related surfaces. They must not be scored as one implementation.

| Surface | What it is | Evidence used |
| --- | --- | --- |
| OmO harness | public monorepo `code-yeongyu/oh-my-openagent`: Core packages plus OpenCode, Codex, and Senpi adapters | `verified/source` at the pin below |
| OmO for Codex ("Light", `lazycodex`) | a Codex plugin inside the same monorepo (`packages/omo-codex`) | `verified/source` |
| OmO Desktop | Electron app: a T3 Code-derived server (`t3`) plus a bundled `omo` runtime (Senpi + OmO) | static inspection of a signed macOS arm64 beta installer; not launched |

| Field | Value |
| --- | --- |
| Harness repository | <https://github.com/code-yeongyu/oh-my-openagent> |
| Harness source pin | [`04ba4a7081e2877057b6ba67e0ce933beb06930e`](https://github.com/code-yeongyu/oh-my-openagent/tree/04ba4a7081e2877057b6ba67e0ce933beb06930e) (2026-11-10, latest tag v5.1.66) |
| Desktop build | OmO 2026.1025.3, macOS arm64, beta channel |
| Desktop runtime | bundle reports `@code-yeongyu/senpi` 2026.10.23-1 and OmO [`v5.1.46`](https://github.com/code-yeongyu/oh-my-openagent/tree/638d30d68837e994ea2edb3a87cb94f40588d991) (`638d30d68837e994ea2edb3a87cb94f40588d991`) |
| Desktop server origin | [T3 Code](https://github.com/pingdotgg/t3code) (MIT), checked at [`cec8d73a1f0fd3bd9a48df88640436e73a04afa3`](https://github.com/pingdotgg/t3code/tree/cec8d73a1f0fd3bd9a48df88640436e73a04afa3) |
| Audit date | 2026-11-10 (Asia/Seoul) |
| Runtime read | 2026-11-13: OmO `638d30d6` (v5.1.46) and Senpi [`2638d524`](https://github.com/code-yeongyu/senpi/tree/2638d524bab2bb87cff874ef1f7e11df16c77643), the commits in the bundle manifest of the Windows desktop build, plus the local records of one session. See [Runtime Behaviour at the Installed Pin](#runtime-behaviour-at-the-installed-pin-2026-11-13) |
| Akra baseline | `6c58243763c2f7ca798a53de27cd427982996fa8` (`prerelease`) |
| Previous Akra audit | 2026-08-08: installed Senpi v2026.7.26, OmO public v4.19.4 plus dev adapter `fd79bf49bd45e2053658c7d52c512d7330037e34`, Akra `0c94f7e8b4549c2358f8b6d4b0c1ebc7e206dd48` |

Desktop limits: the desktop-only code is proprietary, the server is a minified bundle, and the
runtime core ships as bytecode. Desktop claims below are architecture-level facts read from static
strings. They are not runtime behavior. No account data, prompts, sessions, or credentials were
read.

## Material Change Since 2026-08-08

- OmO became a multi-harness product: one harness-neutral Core layer with OpenCode ("Ultimate"),
  Codex ("Light"), and Senpi adapters. The repository says that this refactor is still in
  progress.
- A Codex plugin edition now exists. It extends official Codex through plugin hooks instead of
  owning provider requests.
- A desktop app exists. It is built on a T3 Code fork, but its Codex driver is retired
  (`codexSettingsRetired`, old `~/.codex` data becomes a read-only history source). The bundled
  `omo` runtime calls provider endpoints directly, including the ChatGPT subscription backend
  and Anthropic OAuth.
- Development rate: about 8,700 commits in the 30 days before the audit, mostly from one author
  identity and agent identities (`inferred`: agent-produced). The roadmap states that the code is
  "built for the agent doing the work, not for the human reading it."

## Verdict

OmO is the broadest agent-harness competitor. Its strongest parts are durable child and goal
state, concurrency budgets, stall detection, rich in-harness tooling, and a polished desktop.

It is not structurally more coherent than Akra. The harness has no single state owner. Its state
is spread over per-record JSON files, a DAG WAL, immutable plan revisions, a team store, a plain
`boulder.json`, and a git-backed memory store, with cross-process file locks and mtime staleness
rules. Quality is uneven: some stores are excellent and some are not crash-safe.

The cleanest core in the product, the desktop server's event-sourced orchestration, is inherited
from MIT-licensed T3 Code. It is not OmO-specific design.

Akra keeps its structural edge: one kernel per repository, one reducer, IO-free domain,
enforced boundaries, official Codex as the engine, and reviewed delivery. Akra's real gap is the
same one plan 19 records: post-turn judgment state is not durable across a kernel restart.

## Product Boundary

```text
OmO Desktop (Electron)
  -> t3 server (T3 Code fork, Effect-TS, SQLite event log)
       -> omo runtime daemon (Senpi + OmO, in-process multi-session)
            -> provider APIs directly (ChatGPT backend, Anthropic, xAI, OpenRouter, ...)

OmO for Codex (Light)
  official Codex process
    + plugin hooks (SessionStart, UserPromptSubmit, Pre/PostToolUse, PostCompact, Stop, SubagentStop)
    + MCP servers + agent role files + skills

Akra
  TUI / Desktop / CLI -> Gateway -> repository kernel -> official codex app-server
```

## Harness Core (public source)

### Layering

- Core packages must be harness-neutral. `script/shared-core-extraction-guard.test.ts` enforces
  that Core does not import adapters or hook names.
- Core is not IO-pure. `team-core` and `boulder-state` do file, git, and tmux IO directly. Pure
  parts exist only in islands: `senpi-task` transitions, the DAG scheduler projection, the memory
  reflection machine, and `delegate-core`.
- Akra uses a stricter meaning of core: the domain has no IO, and architecture tests guard the
  dependency direction across at least 90% of Rust sources.

### State and recovery

| Store | Design | Assessment |
| --- | --- | --- |
| ulw-loop plan (`packages/omo-codex/plugin/components/ulw-loop/src/plan-commit.ts`) | each mutation publishes one immutable revision with `link` to `revisions/<n>.json`; an existing number fails (optimistic concurrency); `goals.json` and `ledger.jsonl` are rebuilt views | strongest design; equivalent to an event log with command fencing |
| DAG (`packages/senpi-task/src/dag/store.ts`) | sequenced WAL, torn-tail recovery, checkpoints, fsync by default | crash-safe |
| child tasks (`packages/senpi-task/src/state/`, `store/`) | two explicit enums, pure transition table, terminal states absorb late transitions, per-record lock, tmp+rename, no fsync | good model; durability gap |
| child completion (`packages/senpi-task/src/completion/notifier.ts`) | dedupe on `(task_id, run_epoch)`, `notified_epoch` watermark, redelivery at session start | survives restart, but the watermark is written before delivery, so a kill in that window can lose one notification (`inferred`) |
| team (`packages/team-core`) | FSM `creating -> active -> shutdown_requested -> deleting -> deleted`, mailbox reservation ledger | stored limits (`maxParallelMembers`, wall clock, turns) are not enforced (`inferred`); mtime reclaim can deliver twice (`inferred`) |
| boulder (`packages/boulder-state/src/storage/write-state.ts`) | `writeFileSync`, no tmp+rename, no lock, errors swallowed | not crash-safe under concurrent writers |

The lock code states its own limit: lease expiry permits overlapping bodies, and reclaim is not
atomic (`packages/omo-codex/plugin/components/ulw-loop/src/state-lock.ts`).

### Concurrency and stall control

- Child work runs in per-model lanes plus a global semaphore keyed by `(taskId, runEpoch)`.
  Defaults are 5 per lane and `max(8, 2 x cores)` globally, with depth 1.
- Stale cancels are fenced by the expected run epoch. Epochs are burnt, not reused (anti-ABA).
- Stall rules: the Codex Stop hook stops after 2 resumes with no new ledger entries and writes a
  `.stuck` marker. The same external blocker seen 3 times moves the goal to
  `needs_user_decision`.

### Ideas shared with Akra

Both products independently use epoch fencing for stale completions, lease fencing with owner
tokens, absorbing terminal transitions, and pure transition functions with IO around them.

## OmO for Codex (Light)

This is the most direct comparison: both products use official Codex and neither owns provider
requests.

| Area | OmO for Codex | Akra |
| --- | --- | --- |
| Position | inside Codex as a plugin: 21 hook files, 4 MCP servers, 12 agent roles | outside Codex as an app-server client and supervisor |
| Continuation | Stop hook returns `decision: block` with a resume directive | auto-follow, post-turn evaluation, planning queue |
| Mid-turn death | not recovered; documented as out of scope | kernel reconnects; judgment state is lost (plan 19 P1) |
| Parallel work | Codex `spawn_agent`; one shared checkout by default; worktrees only in team mode with local `--no-ff` merges | slot worktree pool, leases, exact source |
| Delivery | none: no PR creation, CI watching, review polling, remote merge, or cleanup | full reviewed-delivery lifecycle |
| Editing feedback | LSP diagnostics after edits, comment checker, rules injection, spawn role and fan-out guards, evidence receipt gate | Codex defaults only |
| Installer | default `approval_policy="never"` and `sandbox_mode="danger-full-access"`; pre-trusts its own hooks with `trusted_hash`; uninstall does not restore these settings | does not change the operator's Codex configuration |

## OmO Desktop

- Server core (inherited from T3 Code, `verified/source` in the T3 Code pin):
  `apps/server/src/persistence/Migrations/001_OrchestrationEvents.ts` defines an append-only
  event table with a unique `(aggregate_kind, stream_id, stream_version)` index, and
  `002_OrchestrationCommandReceipts.ts` keys receipts by `command_id`. Projections are rebuilt
  from the log. Clients subscribe with a snapshot plus event sequence.
- OmO additions visible in the bundle: per-thread message queue, goal mode, git-ref turn
  checkpoints, PR list and code review, GitHub auto-merge and stacks through `gh`, worktree cleanup
  policies, browser preview with annotation, iOS and Android simulators, computer use, SSH
  environments, a managed relay tunnel, and mobile push.
- Engine recovery is best effort: a turn is resent or reported as lost after an engine restart.
- Telemetry is on by default (PostHog, enumerated fields). It can be turned off with
  `DO_NOT_TRACK=1` or `OMO_DISABLE_TELEMETRY=1`.

## Runtime Behaviour at the Installed Pin (2026-11-13)

This section is a source read of the two repositories at the commits that the Windows desktop
bundle reports, checked against the local records of one real session (about 52 minutes, two Akra
Desktop pull requests and one review of two kernel commits). The installed runtime is bytecode;
the read is of the public source at the same commits. Token figures are in
[Cache and Token Efficiency](../cache-and-token-efficiency.md#second-snapshot-2026-11-13).

### Which layer owns what

| Layer | Repository | Owns |
| --- | --- | --- |
| Senpi host | `code-yeongyu/senpi` (`coding-agent` is about 346k lines) | provider transport and cache key, the default system prompt, compaction, the `eval` tool, goals, todo enforcement, stop hooks, rule injection |
| OmO adapter | `packages/omo-senpi` (about 103k lines) | the `ultrawork` directive, the memory block, the continuation hooks, model profiles |
| Task engine | `packages/senpi-task` (about 64k lines) | child tasks: categories, model resolution, spawn, result delivery |

### Model orchestration

- The orchestrating model chooses a `category` or a named agent as an argument of the `task`
  tool. Code maps the category to a model. A category and an explicit `model` together are
  refused, but a named agent can be given an explicit model.
- Categories and their default models (`senpi-task/src/category/`): `quick` is
  `gpt-6-luna-fast`, `deep-low` is `gpt-6.1-sol` with medium reasoning, `deep-high` and
  `ultrabrain` are `gpt-6-astra`, `unspecified-*` and `architect` are Claude models, and
  `visual-engineering` and `writing` are Gemini. `deep-low` is described as the default lane. A
  child on the wrong lane is told to answer `ESCALATE: deep-high`.
- Exploration agents default to cheap models. The reviewers take a category: the gate reviewer
  defaults to `deep-high`.
- The fallback list of a category task is the rest of the category chain plus the parent's model.
  The local records matched this list exactly.
- Limits: depth 1 by default, 5 concurrent tasks for each model lane, and a global limit. No token
  or cost budget exists for an ordinary child. `max_turns` is parsed and not used.
- Observed: the orchestrator ran on `gpt-6-astra` and all six children on `gpt-6.1-sol`. For the
  gate reviews the orchestrator gave the cheaper model explicitly (`source: "explicit"` in the task
  record) instead of the `deep-high` default.

### Child handoff

- A child is a new session. It gets a five-line identity, the role instructions, and the parent's
  prompt. It does not get the parent transcript, extensions, skills, or context files.
- A child uses the default Senpi system prompt and the parent's tools minus the workflow, team,
  and ask-the-user tools. Its prefix is not the parent's prefix.
- Only the final response returns to the parent, at most 32,000 characters; the rest goes to a
  file. `task_output` returns the last 60 lines by default, at most 30,000 characters, with tool
  names and without tool results.
- The parent does not poll. It ends its turn, and a completion notice wakes it. Notices are
  batched into one injection and deduplicated on `(task_id, run_epoch)`.
- On Windows the mode is always in-process.

### What the harness injects

| Injection | Owner | Trigger | Position | Size |
| --- | --- | --- | --- | --- |
| `ultrawork` directive | OmO | the word `ulw` or `ultrawork` in the input | hidden message at the end | about 31k characters, once for each session and again after each compaction |
| memory block | OmO | each turn | inside the system prompt, pinned to the revision of the first turn | up to 24,576 bytes |
| continuation prompt (ulw-loop, plan) | OmO | the agent settles with open work | hidden follow-up that starts a turn | about 300 characters, at most 8 in a row |
| goal continuation prompt | Senpi | an active goal at the end of a turn, or a 270-second timer | hidden follow-up | about 4.5 KB each time |
| first-turn todo reminder | Senpi | the first request of a session | hidden reminder; the default also forces the `todo` tool | short, once |
| todo-owed backstop | Senpi | a turn that ends with open todos and no tool call | one-sentence reminder that starts a turn | at most 2 in a chain |
| project rules | Senpi | `AGENTS.md` of the working directory and its ancestors | system prompt, up front | up to 64 KiB for each file |
| lazy rule activation | Senpi | a `read`, `edit`, or `write` on a path that a rule matches | suffix of that tool result, once for each session, again after compaction | up to 12,000 characters for each rule |
| stream guard (`ttsr`) | Senpi | repeated output, leaked control tokens, fake tool-call text | aborts the stream and sends a hidden interrupt | short |

- Almost every injection is appended at the end or attached to a tool result. The start of the
  prompt stays stable. This is the same rule that gives the high cache-read fraction.
- The backstop is quiet when it should be: in the observed session it was suppressed 36 times
  (non-interactive child, waiting on a child or a monitor, no open todo) and each suppression
  cost no tokens.
- The OmO Senpi edition does not inject rules itself. Its own guide says that rules are
  intentionally not a Senpi component; the host does it.
- The quality of the delivered work came from the directive and the gate: a plan file, evidence
  files under `.omo/evidence/`, and one independent gate reviewer that must reproduce the claims.
  In the observed session each gate review found one real defect before merge (an unconfirmed tag
  pushed with a branch, and line endings changed by a partial discard). The reviewer prompt asks
  for inputs (a code-review report, a QA matrix) that did not exist, and the reports said so.

### Context control

- Compaction triggers at 70% of the context window (80% above 512k). The Senpi catalog gives
  `gpt-6-astra` a 600,000-token window, so the observed 240k tokens for each response were under
  every trigger.
- The reduction of old tool results without a model call is off for models that use OpenAI remote
  compaction.
- The OmO OpenCode edition has a dynamic pruning option, off by default. The Senpi edition has no
  conversation pruning of its own.

### What this means for Akra

- OmO's delegation is efficient where Akra's is already going: bounded results, no polling, and a
  child that starts clean. The observed cost was in the orchestrator's unreduced history.
- Akra owns none of the provider-side mechanisms and should not. The transferable rules are
  placement (volatile text last) and bounding (what a child returns).
- An independent review step that must reproduce claims found real defects in both products
  during the same days. It is worth a fixed place in the Akra delivery flow.

Evidence gaps for this section: the installed binary was not compared with the source; fallback
during a run, the provider's use of the cache key, and the context window that the installed app
used were not observed; `memory-core`, team mode, the DAG, and the workpool were not read. A dozen
central claims were reopened in the source by the author; the other file references come from a
delegated read and were not each reopened.

## Current Competitive Shape

### Strengths

- Durable child and goal continuation across restarts, with epoch-fenced dedupe.
- Explicit goal and task state machines, and progress-based stall detection.
- Concurrency budgets for many children instead of one global turn.
- In-harness editing feedback and agent role guards.
- Desktop breadth and polish, on top of a clean inherited event-sourced server.

### Limits Akra Can Exploit

- No single state owner; correctness depends on many file locks and mtime heuristics.
- Uneven durability: no fsync on task records, non-atomic `boulder.json`, an optimistic
  notification watermark.
- No reviewed-delivery lifecycle in the Codex edition or the harness.
- The desktop owns a large provider surface, including unofficial subscription backends.
- Unsafe installer defaults that persist after uninstall.
- Very high churn and an in-progress package refactor reduce reproducibility.

## Akra Decisions

### Adopt

- A durable decision record for plan 19 Q3/K5: an append-only SQLite table with a unique sequence
  per stream and command receipts keyed by command id. T3 Code's MIT schema is the reference
  shape; OmO's immutable revision publication is the same idea on files.
- Progress-based stop rules for auto-follow: stop after N follow-up turns with no recorded
  progress, and hand off to the operator after the same blocker repeats.
- Durable completion delivery with a watermark that is written only after delivery is
  acknowledged.
- Last-request and cumulative cache-read fractions with explicit formulas (unchanged from the
  previous audit; see [Cache and Token Efficiency](../cache-and-token-efficiency.md)).

### Reject

- Provider request, auth, cache-key, affinity-header, or fallback ownership.
- Unofficial subscription backends or a retired official Codex path.
- Correctness based on mtime staleness or non-atomic file writes.
- Installers that silently enable full access or pre-trust their own hooks.

### Differentiate

- One kernel per repository with one reducer and an IO-free domain.
- Official Codex as the single engine.
- Planning leases, exact-source worktrees, CI Gate, review handling, rebase merge, and cleanup as
  one durable state machine.
- Safe defaults: the operator's Codex configuration is not changed.

### Watch

- Whether OmO adds PR delivery or CI remediation to the harness or the Codex edition.
- Whether the desktop restores an official Codex driver.
- Completion of the package refactor, and whether Core becomes IO-pure.
- A stable, non-beta desktop release.

## Evidence

- [Harness pin](https://github.com/code-yeongyu/oh-my-openagent/tree/04ba4a7081e2877057b6ba67e0ce933beb06930e)
- [Roadmap and layering](https://github.com/code-yeongyu/oh-my-openagent/blob/04ba4a7081e2877057b6ba67e0ce933beb06930e/ROADMAP.md)
- [Codex edition](https://github.com/code-yeongyu/oh-my-openagent/tree/04ba4a7081e2877057b6ba67e0ce933beb06930e/packages/omo-codex)
- [Codex subagent routing](https://github.com/code-yeongyu/oh-my-openagent/blob/04ba4a7081e2877057b6ba67e0ce933beb06930e/packages/omo-codex/SUBAGENT-ROUTING.md)
- [ulw-loop plan commit](https://github.com/code-yeongyu/oh-my-openagent/blob/04ba4a7081e2877057b6ba67e0ce933beb06930e/packages/omo-codex/plugin/components/ulw-loop/src/plan-commit.ts)
- [Senpi task engine](https://github.com/code-yeongyu/oh-my-openagent/tree/04ba4a7081e2877057b6ba67e0ce933beb06930e/packages/senpi-task)
- [Team core](https://github.com/code-yeongyu/oh-my-openagent/tree/04ba4a7081e2877057b6ba67e0ce933beb06930e/packages/team-core)
- [Boulder state write](https://github.com/code-yeongyu/oh-my-openagent/blob/04ba4a7081e2877057b6ba67e0ce933beb06930e/packages/boulder-state/src/storage/write-state.ts)
- [Core extraction guard](https://github.com/code-yeongyu/oh-my-openagent/blob/04ba4a7081e2877057b6ba67e0ce933beb06930e/script/shared-core-extraction-guard.test.ts)
- [T3 Code migrations](https://github.com/pingdotgg/t3code/blob/cec8d73a1f0fd3bd9a48df88640436e73a04afa3/apps/server/src/persistence/Migrations.ts)

Evidence gaps: desktop runtime behavior was not observed, the runtime core is bytecode, and the
desktop server's OmO-specific source is not public. Claims marked `inferred` came from reading
code paths, not from reproduction.
