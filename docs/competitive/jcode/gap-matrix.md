# jcode To Akra Gap Matrix

This document turns the [jcode v0.43.0 analysis](analysis.md) into Akra decisions. Evidence and
limitations are in [evidence.md](evidence.md). `Ahead` means stronger evidence for the named
dimension at the pinned snapshots, not a total product score.

## Relative Matrix

| Dimension | jcode v0.43.0 | Akra `66333152` | Verdict | Consequence | Evidence |
| --- | --- | --- | --- | --- | --- |
| Product thesis | broad high-skill harness: claimed speed, sessions, customization, memory, swarm | Codex-first client plus durable planning and delivery, but the end-to-end promise is fragmented across docs | jcode ahead in clarity | State and demonstrate the Codex-to-reviewed-integration promise | [product](evidence.md#product-and-release), [Akra](evidence.md#akra-baseline-evidence) |
| Runtime authority | owns daemon, providers, tools, sessions, memory, and swarm | delegates runtime to official `codex app-server`; owns operator workflow | deliberately different; security winner unknown | Keep the narrower owned boundary and expose more native protocol value | [runtime](evidence.md#runtime-architecture), [Akra](evidence.md#akra-baseline-evidence) |
| Startup/input proof | public tables plus executable PTY/PSS scripts; current numbers not pinned to v0.43 | immediate-frame scheduler test and terminal captures, no repeatable benchmark | jcode ahead in discipline; performance winner unknown | Build an Akra evidence contract before making speed claims | [performance](evidence.md#performance) |
| TUI information hierarchy | prioritized negative-space widgets, side panel, diagrams, workspace and swarm instrumentation | strong inline shell, overlays, parallel board, host scrollback, and a coarse completed-item activity line; started/delta/diff/plan/token detail is missing | jcode ahead in density; Akra ahead in host-history contract | Extend the bounded protocol-native execution rail, not a generic widget framework | [TUI](evidence.md#tui-and-visual-system), [Akra](evidence.md#akra-baseline-evidence) |
| Terminal philosophy | custom scrollback and richer internal viewport, with acknowledged smooth-scroll limits | inline rendering preserves host scrollback and tests insertion/restore across terminals | different | Preserve Akra's host scrollback; borrow priority and drilldown patterns only | [TUI](evidence.md#tui-and-visual-system), [Akra](evidence.md#akra-baseline-evidence) |
| Live turn control | soft interrupts, communication, and server-owned session control | active turn input waits; protocol schema contains `turn/steer` but the port does not | jcode ahead in current interaction breadth | Add CAS-safe app-server turn steering | [runtime](evidence.md#runtime-architecture), [Akra gaps](evidence.md#akra-baseline-evidence) |
| Session economics | shared daemon reuses server-owned state; reconnect/reload is central | one TUI with official session catalog and resumed Codex threads; parallel workers add processes | topology differs; cost winner unknown | Measure complete process trees and improve attach/resume projection | [runtime](evidence.md#runtime-architecture), [limits](evidence.md#audit-limits) |
| Active-turn continuity | daemon-owned session runtime is client-independent by design; live survival was not reproduced | official thread history resumes, but connection loss/drop terminates the child and reconnect starts a new child without verified active-turn reattachment | jcode ahead in continuity contract | Define active-turn exit, failure, and restart reconciliation without building a second daemon | [runtime](evidence.md#runtime-architecture), [Akra](evidence.md#akra-baseline-evidence) |
| Planning authority | legacy coordinator plan plus live/migrating DAG with ready, blocked, failed, ownership, artifact, and confidence state | accepted SQLite planning authority, staged drafts, queue, repair, supersession | Akra ahead in operator ownership; jcode richer in graph vocabulary | Keep authority model; adopt explicit dependency/failure evidence where useful | [swarm](evidence.md#swarm-and-planning), [Akra](evidence.md#akra-baseline-evidence) |
| Parallel coordination | recursive spawn, messages, activity age, reports, change alerts, optional worktrees; deep mode permits 1,000 members | fixed pool, leases, session detail, control plane, supervisor and distributor | different; jcode live scale/cost unverified | Add missing activity/evidence fields and explicit resource budgets; do not adopt same-checkout coordination | [swarm](evidence.md#swarm-and-planning), [limits](evidence.md#audit-limits) |
| Isolation | optimistic no-lock same-repo work is supported; docs describe optional worktree roles but automated lifecycle is unverified | one worktree/branch/reviewable slice per lane | Akra ahead in verified default isolation | Make isolation and hotspot ownership visible as a product capability | [swarm](evidence.md#swarm-and-planning), [Akra](evidence.md#akra-baseline-evidence) |
| Delivery | docs assign integration to worktree managers, but automated worktree/delivery lifecycle was not verified | default commit/push/PR/reviewed-range integration and cleanup; explicit parent high-risk opt-in skips review/check gates and may also skip PR automation | Akra ahead in verified default delivery contract; autonomous bypass weakens review evidence | Promote delivery evidence, PR/gate provenance, and critical review response to first-class TUI/Admin state | [swarm](evidence.md#swarm-and-planning), [Akra](evidence.md#akra-baseline-evidence) |
| Long-term memory | automatic extraction/retrieval, embeddings, tools, session search; full hybrid graph partly planned | durable planning, official sessions, direction/task provenance; no semantic memory | jcode ahead in breadth | Add missing provenance links before embeddings | [memory](evidence.md#memory-providers-and-extensibility) |
| Provider breadth | many OAuth, API, local, compatible, and runtime adapters | Codex only through official runtime | jcode ahead by choice | Do not compete here; Codex specialization is the wedge | [providers](evidence.md#memory-providers-and-extensibility), [Akra](evidence.md#akra-baseline-evidence) |
| Extensibility | tools, MCP, hooks, provider profiles, side panels, self-development | Codex capabilities plus application ports and fixed operator surfaces | jcode ahead in user customization | Add only capability/status discovery that respects app-server authority | [providers](evidence.md#memory-providers-and-extensibility) |
| Browser Admin | no verified product web Admin in audited source | Axum/Askama read-only parallel dashboard, JSON API, review center, tasks, metrics, game diorama | Akra ahead in web observability, not yet a full control plane | Add lifecycle metrics and guarded application-service actions | [remote](evidence.md#desktop-and-remote-control), [Akra](evidence.md#akra-baseline-evidence) |
| Desktop | large custom Rust prototype, public architecture still proposed | no desktop client | jcode ahead in investment; maturity uncertain | Do not divert from TUI/Admin until their core loop is measurably superior | [desktop](evidence.md#desktop-and-remote-control) |
| Remote control | gateway/mobile directions, daemon/debug interfaces, hooks | Admin, Telegram, GitHub delivery/review, CLI/JSON tools | Akra ahead for verified current operations | Unify them around the same structured lifecycle and guarded commands | [remote](evidence.md#desktop-and-remote-control), [Akra](evidence.md#akra-baseline-evidence) |
| Safety surface | wide provider/tool/auth surface, permission UI, pre-tool hook, design-stage ambient safety | official runtime, bounded interactive approvals, unattended decline, identity-gated GitHub writes | structural difference; comparative safety unverified | Make fail-closed policies and delivery audit visible; add a shared threat model before claiming superiority | [providers/hooks](evidence.md#memory-providers-and-extensibility), [Akra](evidence.md#akra-baseline-evidence) |
| Quality instrumentation | extensive tests, benchmarks, ratchets, cross-platform jobs | strong Rust tests, native/TUI layering and terminal validation, less performance budgeting | jcode ahead in breadth | Add measurable performance and maintainability ratchets selectively | [quality](evidence.md#quality-and-maintainability) |
| Quality-ratchet health | four checked-in ratchets fail locally at the clean v0.43 tag | current native gate is broad; this audit did not rerun all CI | jcode weakness; Akra status not compared | Treat green artifacts, not configured jobs, as proof | [guard output](guard-results.txt), [quality](evidence.md#quality-and-maintainability) |

## Adopt

### 1. Performance As A Versioned Contract

Adopt the measurement discipline, not jcode's published numbers.

Akra needs separate metrics for:

- process spawn to first meaningful shell frame;
- first frame to visible prompt echo;
- process spawn to ready-to-submit after app-server startup;
- submit to first protocol event and first assistant delta;
- idle, streaming, and parallel-pool PSS for the full process tree;
- p50/p95 frame work and event backlog under a fixed synthetic stream.

Raw samples, versions, auth state, warm/cold state, terminal geometry, process tree, and environment
stamp are mandatory. A rendered summary without the raw artifact is not evidence.

### 2. Priority-Based TUI Instrumentation

Adopt jcode's rule that scarce terminal space has explicit priorities and minimum useful sizes.
Map it to Akra facts:

1. active approval or failure
2. live command/patch/plan activity
3. context pressure and active model/effort
4. current planning task and continuation reason
5. parallel lane status and selected lane detail
6. diagnostics and secondary hints

Wide terminals may show a rail. Narrow terminals should collapse lower-priority facts into one
status line or an existing detail overlay. Do not render empty boxes to preserve a dashboard shape.

### 3. Rich Parallel Activity

Adopt fields that help the operator decide whether a lane is healthy:

- lifecycle state age and last activity age as different values;
- current app-server item/tool class;
- task/role label;
- bounded validation and completion evidence;
- failure reason and blocked dependency;
- report-to owner and integration state;
- file/hotspot ownership declared at assignment time.

These belong in the existing parallel session detail and runtime event contracts, not in a new
agent messaging system.

### 4. Nonblocking Coordination

The coordinator and TUI must remain usable while workers start, wait, report, refresh official
session truth, or await delivery. Extend existing control-plane effects and event projections rather
than blocking an inbound handler on worker completion.

### 5. Outcome-Oriented Guardrails

Adopt budgets where Akra has measurable risk:

- startup/input/PSS regression budget;
- TUI frame/event backlog budget;
- LLM-facing TUI file-size ratchet consistent with the current agent guide;
- production panic and swallowed-boundary-error report;
- dependency direction check for `domain`, `application`, `core`, and adapters.

The gate must pass at the release commit. A stale baseline is worse than no baseline because it
creates false confidence.

## Reject

### Multi-Provider Runtime

Do not add provider transports, subscription OAuth imports, model catalogs, or provider-specific
tool translation to Akra. That would duplicate the official runtime, expand the credential attack
surface, and erase the Codex-first position.

### Local Agent Tool Runtime

Do not build a parallel registry for file, shell, browser, LSP, MCP, or memory tools. Project useful
app-server events and capabilities through ports. Add an outbound port only for a real Akra-owned
boundary such as GitHub delivery, planning persistence, or Telegram.

### Same-Checkout Swarm

Do not trade worktree isolation for file-read notifications and agent-to-agent conflict repair.
Notifications are useful warnings, not isolation. Akra's one-lane/one-worktree rule is a stronger
default for reviewed integration.

### Agent Messaging As Coordination Authority

Do not make DMs or broadcast channels the source of task truth. Accepted planning, leases, runtime
events, completion evidence, and integration state remain authoritative and inspectable by the
operator.

### Full Semantic Memory Graph

Do not add embeddings or automatic personal memory until official-session search and planning
provenance have a measured failure. A memory system would require privacy controls, correction,
expiry, conflict resolution, packaging, resource budgets, and visible provenance.

### Custom Scrollback Replacement

Do not replace host terminal history with an in-app transcript viewport. The inline scrollback
contract is an Akra differentiator. Rich live state should remain bounded and must not replay as
permanent chrome.

### Desktop And Mobile Expansion

Do not start a native desktop or mobile shell while the TUI lacks live protocol detail and Admin
lacks real operational metrics. Admin already supplies the cross-device operator surface with much
lower runtime duplication.

### Self-Development Hot Reload

Do not optimize for agents rewriting and hot-reloading Akra itself as a primary product workflow.
Use normal worktrees, validation, review, releases, and rollback. Developer iteration speed should
improve through module boundaries and build profiles, not a second runtime lifecycle.

## Differentiate

### Codex Protocol Lead

Akra should be the wrapper that exposes useful official app-server features first and accurately.
The current schema already contains behavior that the product does not project.

Immediate targets:

- `turn/steer` with `expectedTurnId` compare-and-set semantics;
- `item/started` lifecycle;
- command output delta;
- file patch update;
- aggregated turn diff update with bounded summary and selected full-diff inspection;
- plan update;
- token/context usage update.

This is a stronger differentiator than generic provider choice because it compounds with every
official Codex improvement.

### Delivery, Not Mere Completion

A worker is not done when it emits a summary. Akra should show and enforce the default path:

```text
assigned -> running -> reported -> commit ready -> source range frozen
-> configured local validation clear or not-required -> source published -> PR open
-> trusted-CI validation clear or not-required
-> frozen-head approval + CLEAN + required checks clear -> integration applied
-> integration ref pushed and remotely verified -> integrated
-> PR closed -> source/slot cleaned
```

The explicit high-risk autonomous paths must expose their branches rather than imitating reviewed
state:

```text
commit ready -> source range frozen -> local validation clear or not-required
-> source published -> PR open -> trusted-CI validation clear or not-required
-> review/check gates skipped (parent policy recorded) -> integration applied
-> integration ref pushed and remotely verified -> integrated -> PR closed -> source/slot cleaned

commit ready -> source range frozen -> local validation clear or not-required
-> source published -> trusted-CI validation clear or not-required
-> PR and review/check gates skipped (parent policy recorded) -> integration applied
-> integration ref pushed and remotely verified -> integrated -> source/slot cleaned
```

Autonomous policy never bypasses the validation policy. Direct mode remains blocked when a required
trusted-CI check cannot run without PR context.

Every state should have an owner, timestamp, failure reason, and drilldown. This is where Akra can
be more trustworthy than a broad swarm harness.

These sequences describe the runtime distributor. Manually managed feature lanes continue to follow
the repository's local rebase plus base fast-forward policy; the two delivery paths must not share
misleading provenance labels.

### Operator-Owned Intent

Accepted planning remains the durable source of intended work. The operator should be able to see:

- which direction and task caused a turn;
- what was superseded;
- why an automatic continuation was selected;
- which validation and review evidence, or explicit review-skipped policy, closed the task;
- which remaining task follows.

### Operational Game Board

Use the Admin diorama to express real progress, not decoration:

- workers occupy stations according to actual lifecycle state;
- blockers and review waits have distinct visible states;
- completed delivery changes the project/company progression;
- throughput, success rate, queue wait, and integration latency come from persisted events;
- no fabricated percentage or API speed appears when data is absent.

## Implementation Slices

These slices are independently reviewable. File lists are ownership hints, not permission to mix
all rows into one PR.

### P0: Native Performance Evidence Contract

**Owned boundary**

- a benchmark driver under `scripts/`
- the existing native-validation capture/manifest schema and `docs/validation/` artifact index
- focused Rust integration tests and existing native-validation script tests

**Contract**

- run cold-start and warm-resume modes for at least 30 attempted samples each;
- drive a fixed 80x24 PTY and record terminal query handling;
- record first frame, first input echo, ready-to-submit, first event, and first assistant delta;
- define the process set as Akra plus owned descendants, record shared-daemon attribution, and avoid
  double counting across clients;
- sample Linux PSS, macOS physical-footprint/RSS evidence, and Windows private-working-set evidence
  for idle, active stream, and three-slot parallel profiles; retain the platform-specific metric
  name and never compare unlike memory metrics as one number;
- retain startup failures, timeouts, and censored memory samples in raw output instead of dropping
  them from the denominator;
- compute p50/p95 with a checked-in deterministic nearest-rank rule and report successful/attempted
  sample counts beside every percentile;
- emit one raw JSON artifact plus a deterministic summary;
- include OS, kernel, CPU, memory, terminal/PTY, git SHA, Akra version, Codex version, auth state,
  app-server warm state, command, environment overrides, and failures;
- extend the existing native-validation artifact schema rather than creating a second evidence
  format;
- establish an Akra baseline ratchet before cross-product thresholds.

**Required proof**

- deterministic parser/summary tests;
- a real Linux baseline artifact;
- a representative macOS terminal baseline artifact;
- Windows Terminal/PowerShell and Windows Terminal/WSL profiles when terminal semantics apply;
- explicit evidence that cross-OS memory values are not normalized into a false comparison;
- a documented reason for every incomparable competitor sample.

**Do not claim**

- that Akra is faster than jcode until current binaries run under the same boundary and environment.

### P0: Protocol-Native Live Execution Rail

**Owned boundary**

- `src/domain/conversation*.rs`
- `src/application/service/conversation_runtime_event.rs`
- `src/core/app/turn_stream.rs`
- `src/adapter/outbound/app_server/protocol/turn_notifications.rs`
- `src/adapter/inbound/tui/app/conversation_model/turn_activity.rs`
- `src/adapter/inbound/tui/app/shell_presentation/status_panels/`

**Contract**

Preserve the shipped completed-item activity path: typed command/file-change events, current/last
counts, latest summary, and the compact tail line. Extend that state with typed forms of:

- `item/started`
- `item/commandExecution/outputDelta`
- `item/fileChange/patchUpdated`
- `turn/diff/updated`
- `turn/plan/updated`
- `thread/tokenUsage/updated`

Reduce item events by item ID and turn-level events by turn ID into bounded state. The rail shows
recent action, active count, changed-file count, diff stats, plan step, and context pressure in three
to five rows. A selected drilldown renders the complete retained aggregated diff, with an explicit
truncation state when the provider payload exceeds the enforced memory bound. Raw deltas and full
diffs must not flood permanent scrollback. Completion writes one stable transcript summary.

**Required proof**

- protocol fixtures for identity, ordering, bounds, and schema drift;
- coalescing and memory-bound reducer tests;
- context threshold tests;
- diff replacement, size-bound, truncation, and selected-drilldown tests;
- wide and narrow snapshots;
- vt100/inline recorder coverage;
- real-terminal capture before integration.

### P0: Active Turn Steering

**Owned boundary**

- `src/application/port/outbound/interactive_turn_runtime_port.rs`
- `src/application/service/conversation_service.rs`
- `src/adapter/outbound/app_server/`
- `src/core/app/`
- `src/adapter/inbound/tui/app/turn_submission_runtime.rs`

**Contract**

- running `Enter` sends input to the current app-server turn using `threadId` and
  `expectedTurnId`;
- the active streaming connection owns a control channel so ordering and correlation remain on the
  same runtime path;
- success clears the draft exactly once;
- stale turn, non-steerable turn, rejection, disconnect, and transport failure preserve the draft
  and show an actionable state;
- steering is not counted as a new planning task or continuation turn.

**Required proof**

- fake app-server serialization and correlation tests;
- stale/non-steerable/failure preservation tests;
- core reducer tests;
- running-turn TUI snapshots and terminal capture;
- no regression to approval input ownership.

### P0: Active Turn Exit And Restart Recovery

**Owned boundary**

- app-server connection/runtime lifecycle
- conversation application service and core recovery state
- new recovery-journal port and SQLite adapter
- TUI shutdown/startup flow
- parallel worker supervision and persisted session detail

**Contract**

- before `thread/start`, durably write a new-thread intent containing workspace, catalog watermark,
  requested thread metadata, request nonce, and session kind, then CAS-bind the returned thread ID;
- before `turn/start`, durably write a start intent containing thread ID, preceding-turn identity,
  input fingerprint, request nonce, and session kind;
- CAS-bind the returned turn ID to that intent before projecting a main or parallel turn as running,
  and persist last-observed and terminal events before projecting them;
- after a crash during `thread/start`, reconcile the recent official-session catalog using only the
  workspace, time/watermark, and thread metadata it actually exposes; no input exists at this stage,
  so bind only a unique catalog match and otherwise remain unknown;
- after a crash between remote acceptance and turn-ID binding, reconcile `thread/read` against the
  preceding turn and input fingerprint; bind only a unique match and otherwise retain an explicit
  unknown state;
- on controlled TUI exit with a running turn, offer bounded wait, protocol interrupt-and-exit, and
  explicit force-exit paths; never silently present child termination as successful completion;
- on transport loss, child death, or process exit, mark the turn `recovery_pending` with the last
  observed identity before starting a replacement child;
- after restart, read official thread truth and apply a terminal result exactly once when available;
  otherwise mark the prior turn interrupted or unknown and require an explicit operator decision;
- never submit a replacement turn automatically while a start intent or prior outcome is unknown;
- apply the same terminal/unknown distinction to parallel workers before lease recovery or retry;
- do not introduce an Akra daemon or claim client-independent continuation unless an official attach
  contract is implemented and reproduced.

**Required proof**

- fake-runtime tests covering wait, interrupt, force exit, transport loss, child death, and restart;
- crash-boundary tests before intent persistence, after RPC acceptance but before ID binding, after
  binding, and around last-event/terminal persistence;
- unique-match, no-match, and ambiguous session-catalog and `thread/read` reconciliation fixtures;
- main and parallel recovery tests proving no automatic duplicate turn or duplicate completion;
- TUI shutdown/restart snapshots with explicit unknown and recovered states;
- a real child-kill/restart capture against the pinned app-server version.

### P0: Frozen-Source Validation Evidence

**Owned boundary**

- new domain validation-evidence and policy types
- new application validation service and outbound runner/checks port
- host subprocess and GitHub-check adapters
- parallel session detail, distributor readiness, TUI, and Admin projections

**Contract**

- treat the current planning-file-change `validation_summary` as legacy context, never as proof that
  tests or checks ran;
- load command and required-check policy from operator-owned authority or the frozen integration-base
  revision, never from candidate-controlled configuration; candidate policy changes cannot authorize
  their own validation;
- run configured local validation from a host-owned runner after the worker stops and against a clean
  worktree at the exact frozen source SHA, or a CAS-bound re-freeze candidate; alternatively consume
  an explicitly trusted CI check bound to that exact SHA;
- evaluate configured local validation before publication and mark it clear or not-required; collect
  trusted CI only after lease-bound publication/PR creation, and block integration until every
  required phase is clear;
- treat candidate code, build scripts, and test binaries as untrusted: scrub credentials and ambient
  environment, deny network by default, confine writes to the candidate worktree plus private temp
  and cache roots, and enforce timeout, output, memory/CPU, and child-process limits with process-tree
  termination;
- report local validation unavailable on platforms without the required sandbox instead of running
  candidate code directly on the operator host;
- distinguish local commands from remote checks and store command/check identity, source SHA, start
  and finish time, exit/conclusion, runner/tool versions, artifact reference, and content hash;
- reject worker prose, an unbound artifact, a dirty or moved source, and unknown/failing required
  validation as delivery readiness;
- invalidate prior validation evidence whenever the source tip or frozen range changes;
- keep a bounded human summary for presentation without deriving gate state from that summary.

**Required proof**

- fake runner/check adapter tests for success, failure, timeout, missing, malformed, and stale-SHA
  evidence;
- clean-worktree/source-identity, base-bound policy, candidate policy-tampering, and command-policy
  tests;
- adversarial runner tests for credential/environment reads, outside-root writes, network access,
  forked children, resource exhaustion, oversized output, and incomplete process-tree cleanup;
- crash recovery between execution, artifact persistence, and readiness transition;
- distributor tests proving missing/failing validation blocks and exact-SHA success unblocks;
- TUI/Admin projections and one real frozen-commit validation artifact.

### P1: Authoritative Delivery Outcomes

**Owned boundary**

- domain delivery-attempt outcome and failure-classification types
- planning-authority port, SQLite store, and migration
- application outcome command/CAS service
- distributor recovery/failure classification
- TUI, CLI, and Admin outcome projections and commands

**Contract**

- give each delivery attempt an immutable identity linked to its task and any prior attempt;
- define terminal outcomes as `integrated`, `terminal_failed`, or `canceled`, with skipped or
  superseded work reported separately;
- finalize `integrated` only after fetching or inspecting the canonical remote integration ref and
  verifying it equals the expected applied commit; local application and push are provisional states;
- track PR close, source-branch deletion, worktree cleanup, and slot cleanup as post-integration
  finalization; their failure remains recoverable and never rewrites a verified `integrated` outcome;
- keep retryable `blocked`, automatic-retry exhaustion, and internal `failed` labels nonterminal
  while manual recovery remains possible;
- allow `terminal_failed` only from an application-owned non-retryable classification or an
  authenticated explicit abandon command, recording actor, reason, attempt, and timestamp;
- finalize an attempt once through compare-and-set; a later reopen creates a linked attempt instead
  of rewriting the old outcome;
- expose retries within an attempt separately from reopen attempts and final outcomes.

**Required proof**

- transition tests for retryable, non-retryable, abandon, cancel, integrate, and reopen paths;
- boundary tests for local apply, push failure, remote-ref mismatch, remote verification, and
  post-integration cleanup recovery;
- stale-version, duplicate-command, concurrent-finalization, and crash-recovery tests;
- persistence migration and round-trip tests;
- TUI/CLI/Admin projection and authorization tests;
- one audit trail from blocked recovery or abandon through a final immutable attempt outcome.

### P1: Admin Operational Metrics

**Owned boundary**

- new `src/domain/operational_metrics.rs`
- new application-owned outbound metrics port
- SQLite lifecycle aggregation adapter
- `src/adapter/inbound/admin_api/akra_dashboard.rs`
- Admin JSON/templates and dashboard script

**Contract**

Persist and aggregate:

- daily integrated and explicitly finalized terminal-failed delivery attempts;
- delivery-attempt success rate;
- queue wait;
- dispatch to running;
- running to reported completion;
- commit ready to integrated;
- p50 and p95 by interval;
- lifecycle stage `n/N` for selected work.

The current `None` and `unmeasured` values remain honest until data exists. The UI must not derive
metrics by parsing human-readable event summaries.

Metric semantics are part of the contract:

- store timestamps in UTC and calculate daily buckets in an explicit configured IANA timezone,
  defaulting to UTC;
- consume immutable outcomes from Authoritative Delivery Outcomes; use finalized `integrated +
  terminal_failed` attempts as the success-rate denominator, and report canceled/skipped work
  separately;
- count each attempt's final outcome once while exposing within-attempt retries and linked reopen
  attempts as separate measures;
- exclude still-running/censored work from completed-duration percentiles while reporting its count;
- use nearest-rank percentiles, always show sample count, and withhold p95 until at least 20 completed
  samples exist.

**Required proof**

- injected-clock tests across day boundaries;
- timezone, attempt/retry/reopen, cancellation, censoring, percentile, and empty-store tests;
- Admin JSON and template tests;
- desktop and mobile Playwright captures;
- game-board build and visual checks.

### P1: Guarded Admin Control Actions

**Owned boundary**

- application control-plane commands and ports
- authenticated Admin POST handlers and forms
- Admin security, API, and audit-event projections

**Contract**

- reuse Authoritative Delivery Outcomes commands for terminal abandon/reopen, and define the
  remaining application-owned commands for distributor pause/resume, blocked-delivery retry, review
  handoff, and targeted cleanup recovery;
- expose only those commands through Admin and make them reusable by TUI/CLI;
- require authenticated session, CSRF protection, explicit target/version, and idempotency or
  compare-and-set semantics;
- persist an actor, request, result, and resulting lifecycle event;
- never mutate SQLite, git, a worktree, or GitHub directly from an HTTP handler;
- use confirmation for destructive or remote-write actions and preserve honest unavailable states.

**Required proof**

- authorization, CSRF, stale-version, idempotency, and concurrent-action tests;
- application-service transition tests shared with non-Admin surfaces;
- Admin API/template tests and desktop/mobile Playwright captures;
- a real pause/resume or blocked-retry audit trail from request through projection.

### P1: Parallel Activity And Completion Evidence

**Owned boundary**

- parallel session-detail domain and store
- control-plane and supervisor projection
- TUI parallel detail
- Admin task/agent detail

**Contract**

- preserve the already-shipped lifecycle history, completion/review/cleanup states, blocked reason,
  conflict-file projection, and distributor timeline;
- distinguish lifecycle age from last app-server activity;
- store bounded current app-server item/tool activity and declared hotspot ownership;
- project the exact-SHA gate and bounded summary produced by Frozen-Source Validation Evidence;
  preserve the old free-form field only as labeled legacy context;
- warn before allocation when active slices overlap named hotspots;

**Required proof**

- persistence and recovery tests;
- concurrent allocation collision tests;
- supervisor/TUI/Admin projection tests;
- a real two-lane delivery capture.

### P1: Critical Review Response Loop

**Owned boundary**

- `src/application/service/github_review_poller_service.rs`
- GitHub review port/adapter and identity guard
- parallel delivery claim, queue revision, and frozen-source state
- review-response turn and Frozen-Source Validation Evidence services
- Admin review center and TUI review projection

**Contract**

- load unresolved inline threads with current diff context and resolution state;
- represent reviewer body, linked URLs, and diff context as typed untrusted data; never interpolate
  them into system/developer instructions, shell commands, validation policy, or task authority;
- classify each comment as valid, stale, incorrect, or out of scope with a recorded rationale;
- accept a valid-fix transition only while the delivery record is still in review wait; acquire its
  claim and compare the expected frozen tip, remote branch, PR head, and thread version;
- run a scoped review-response turn in the owning worktree and create a separate clean commit when
  practical;
- persist a re-freeze intent with old tip, candidate tip, and idempotency key, immediately
  invalidating approval, required-check, clean-merge, and validation readiness from the old tip;
- run any policy-required local validation against the exact candidate before publication, then push
  with a lease bound to the old remote tip;
- after the remote branch and PR head match the intent, collect any required trusted-CI evidence
  against the candidate and atomically supersede the old frozen-source revision only when the full
  validation policy is clear; crash recovery must preserve this ordering;
- require fresh default approval, CLEAN, and required-check gates against the newly frozen PR head;
- fail closed if integration has started, the source moved, or another response won the CAS;
- run response turns without GitHub credentials and under the owning task's existing sandbox and
  scope; reviewer requests cannot expand tools, network access, file ownership, or accepted intent;
- for stale or incorrect feedback, preserve code and post a concise evidence-backed rationale;
- reply or resolve only after the intended GitHub identity and repository target are verified;
- keep comment, decision, commit, validation, reply, and final thread/check state linked and
  idempotent;
- never treat reviewer text as an instruction that bypasses architecture, safety, or operator intent.

**Required proof**

- fixtures for unresolved/resolved, outdated diff, valid, incorrect, and duplicate thread cases;
- adversarial fixtures for prompt injection, scope escape, malicious patch context, credential
  requests, shell text, URLs, and attempts to rewrite task or validation policy;
- identity mismatch and remote-write failure tests;
- competing-response and integration-start race tests;
- retry/idempotency tests across validation, re-freeze intent, push, queue-CAS, and reply boundaries;
- tests proving old-tip approval, checks, clean state, and validation cannot authorize the new tip;
- one real PR trail proving comment to decision to commit/reply to refreshed final state.

### P2: Official Session Search And Provenance

**Owned boundary**

- app-server session catalog port/adapter
- session application service and core projection
- planning provenance query
- TUI session browser and Admin session detail

**Contract**

- preserve the shipped query, project filter, local paging, preview, source/model/status/branch
  display, and session selection behavior;
- add provider-cursor `load more` without duplicating or reordering existing results;
- add explicit time-range filtering;
- show the accepted direction/task and delivery result associated with a session;
- classify imported, non-resumable, and official resumable sessions, with non-resumable context
  read-only;
- do not add embeddings in this slice.

**Required proof**

- provider-cursor, deduplication, time-filter, provenance, classification, and stale-session tests;
- cross-repository isolation tests;
- TUI narrow/wide snapshots;
- provenance round trip from accepted task to integrated PR.

## Recommended Order

1. Establish the performance evidence contract when one scripts/validation lane can own the
   existing capture schema without overlapping another live lane.
2. Implement active turn steering as the first app-server/core control-path change.
3. In the same ownership lane after steering, make controlled exit, child loss, and restart reconcile
   active main and parallel turns without automatic duplicate submission or completion.
4. In a disjoint protocol/TUI lane, extend the live execution rail with deltas, context, and full-diff
   drilldown; this can proceed alongside steps 2-3 as a separate PR.
5. Replace planning-file summaries with frozen-source validation evidence and a real delivery gate.
6. Close the critical GitHub review-response loop, including crash-safe source re-freeze and fresh
   validation/review/check gates.
7. Add authoritative delivery-attempt outcomes before building metrics over them.
8. Add Admin operational metrics and guarded control actions through shared application services.
9. Add the missing parallel activity evidence and hotspot collision warnings.
10. Add session provider-cursor/time/provenance gaps before considering semantic memory.

## Success Audit

This comparison has produced value only when later Akra evidence proves all of the following:

- users can see, steer, and inspect the retained diff of a running official Codex turn;
- controlled exit, child failure, and restart reconcile a prior active turn without duplicate work;
- first-frame/input/readiness/stream and platform-labeled process-tree memory performance is
  versioned and repeatable;
- TUI execution state is dense, bounded, responsive, and does not pollute host scrollback;
- required validation is host-run or trusted-CI evidence bound to the exact frozen source SHA;
- every parallel lane exposes activity, validation, PR presence, review/check state or explicit
  skipped-gate policy, integration, and cleanup truth;
- valid review feedback has a crash-safe source revision with fresh validation/review/check gates,
  plus a traceable decision, commit or rationale, reply, and final thread state;
- operational failure rates count only authoritative finalized `integrated` or `terminal_failed`
  delivery attempts;
- Admin's game progression is driven by stored lifecycle data and guarded actions use shared
  application services;
- no multi-provider runtime, same-checkout swarm, or speculative memory graph displaced those goals.
