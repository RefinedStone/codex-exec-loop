# Agent Canvas To Akra Gap Matrix

This matrix turns the [v1.2.1 analysis](analysis.md) and [evidence ledger](evidence.md) into
Akra-relative decisions. It does not treat browser surface area or provider count as goals. Accepted
work must strengthen Akra's Codex-first operating and delivery position and remain reviewable as a
small ownership slice.

## Relative Matrix

| Capability | Agent Canvas v1.2.1 | Akra baseline | Decision | Priority | Evidence |
| --- | --- | --- | --- | --- | --- |
| Selected-session inspector | cohesive chat, files/diffs, observed terminal, browser screenshot, task/planner tabs | native inline conversation plus focused overlays and Admin views | adopt only activity/provenance cohesion | P0/P1 | [Canvas](evidence.md#selected-conversation-ux), [Akra](evidence.md#akra-baseline-evidence) |
| Multi-session overview | filterable conversation rail with coarse activity state | real agent/task/worktree/delivery topology, but no bounded current tool/activity | extend Akra's topology | P0 | [Canvas](evidence.md#selected-conversation-ux), [Akra](evidence.md#akra-baseline-evidence) |
| Codex protocol | released embedded-core ACP relay; important end-to-end losses | direct app-server adapter with schema classification and incomplete rich-event reduction | preserve and measure the structural advantage | ongoing | [ACP](evidence.md#released-codex-acp-path), [Akra](evidence.md#akra-baseline-evidence) |
| Permissions | outer confirmation off; ACP bridge selects option zero | direct boundary, but only accept/decline; file-change approval and MCP elicitation fail closed without forms | reject auto-selection; improve Akra choice fidelity | invariant/P1 | [ACP](evidence.md#released-codex-acp-path), [Akra](evidence.md#akra-baseline-evidence) |
| Parallel isolation | attached workspace defaults local; scratch falls back to worktree request; no cleanup found | leased worktree per parallel lane | differentiate on delivery authority | shipped | [Canvas](evidence.md#conversation-creation-reconnect-and-workspaces), [Akra](evidence.md#akra-baseline-evidence) |
| Git delivery | pull/push/PR commands become agent prompts | shipped frozen range, review/check gates, serialized integration, remote verification, cleanup; exact-SHA validation evidence planned | differentiate without overstating current validation | shipped plus existing P0/P1 | [Canvas](evidence.md#selected-conversation-ux), [Akra](evidence.md#akra-baseline-evidence), [planned validation](../jcode/gap-matrix.md#p0-frozen-source-validation-evidence) |
| Remote backends | browser registry and health for local/remote/cloud | loopback Admin, Telegram control plane | adopt as read-only Akra nodes later | P2 | [Canvas](evidence.md#backend-registry-and-telemetry), [Akra](evidence.md#akra-baseline-evidence) |
| Automation | schedule, webhook, durable run, conversation/log links, partial UI | no schedule/webhook/run ledger | adopt narrowly for reviewed code-change intake | P1 | [Canvas](evidence.md#automation-114), [Akra](evidence.md#akra-baseline-evidence) |
| Automation correctness | replay dedup missing; six-state backend/four-state UI mismatch | no corresponding domain yet | avoid the copied defects | P1 invariant | [Canvas](evidence.md#automation-114), [Akra](evidence.md#akra-baseline-evidence) |
| Game operations | no comparable fleet game | diorama animates work independent of relevant events | correct Akra truthfulness gap | P0 | [Akra](evidence.md#akra-baseline-evidence) |
| Credentials | backend keys in browser storage; broad ACP child environment | filtered app-server child environment; no remote registry | reject browser ownership | invariant | [Canvas](evidence.md#backend-registry-and-telemetry), [Akra](evidence.md#akra-baseline-evidence) |
| Interactive performance | no comparable evidence | no complete native performance artifact yet | extend the existing evidence contract | P0 dependency | [limits](evidence.md#audit-limits), [planned contract](../jcode/gap-matrix.md#p0-native-performance-evidence-contract) |

## Adopt

### 1. Current Activity As Fleet Truth

Canvas demonstrates the value of detailed command, file, reasoning, and task state inside one
selected conversation. Akra already has the more valuable fleet topology, selected `:peek`,
validation, review, delivery, and cleanup detail. The missing piece is a bounded current-activity
envelope that travels from official app-server events into the same parallel truth.

Adopt:

- a typed activity kind and phase rather than a free-form status string;
- the current app-server item identifier and a bounded operator summary;
- last-activity time separate from lifecycle age;
- an authority-assigned monotonic runtime-event sequence so Admin animation and clients can identify
  relevant real changes;
- coalesced persistence and terminal flush rather than one SQLite write per output delta;
- the same projection in TUI, Admin JSON, and selected detail.

This amends and elevates the existing jcode
[Parallel Activity And Completion Evidence](../jcode/gap-matrix.md#p1-parallel-activity-and-completion-evidence)
slice. It must not become a second event store, transcript model, or inspector.

Current Akra conversation activity contains completed command and file-change summaries only. Item
start, command deltas, MCP/tool progress, and reasoning require the existing jcode
[Protocol-Native Live Execution Rail](../jcode/gap-matrix.md#p0-protocol-native-live-execution-rail)
to retain typed events first. The parallel activity slice consumes that domain output; it does not
reparse raw app-server JSON.

### 2. Durable Trigger And Run Provenance

The useful Automation pattern is not arbitrary workflow execution. It is a durable distinction
between a trigger definition and a specific run, with status, timestamps, input metadata, linked
conversation, command/log evidence, cancellation, and recovery.

Adopt for Akra code-change intake:

- schedule and signed event triggers;
- enable, disable, run-now, cancel, and immutable attempt history;
- operator-authored task templates with typed parameters;
- trigger-specific idempotency before task creation;
- separate intake disposition, automation run/attempt state, and authoritative delivery outcome;
- exhaustive run state for skipped, cancelled, timed out, failed, completed, and unknown external
  state without redefining delivery success;
- one provenance chain from trigger to accepted task, lease/session, validation, PR, delivery
  outcome, and cleanup;
- watchdog recovery with a recorded reason, not silent status correction.

Do not pass a raw webhook payload as an agent prompt. The payload is untrusted data and can only
populate bounded fields accepted by an operator-owned template.

Automation must reference the existing jcode
[Authoritative Delivery Outcomes](../jcode/gap-matrix.md#p1-authoritative-delivery-outcomes)
rather than adding `integrated` or `terminal_failed` to an automation enum. Intake can be accepted
before that slice lands, but a terminal reviewed-delivery trace cannot.

### 3. Read-Only Remote Node Awareness

Canvas verifies endpoint health, backend association, and degradation UI. It is reasonable to infer
that this can reduce operator friction, but the effect was not measured. Akra should borrow that
operational shape later, after local delivery outcomes and automation provenance are authoritative.

The first remote contract is read-only:

- Akra node identity, version, capabilities, and workspace scope;
- health, last-success time, snapshot age, and explicit stale state;
- selected task/session/delivery summaries from the same application projection;
- mutually authenticated HTTPS identity and version negotiation;
- credentials in an OS-private or server-side store, never browser storage.

No remote mutation, provider switching, or ACP registry belongs in the first slice.

### 4. Selected-Run Deep Links

Canvas's run-to-conversation and log links are a good information architecture pattern. Akra should
go further by using stable domain identities:

```text
trigger -> run -> accepted direction/task -> lease -> agent session -> app-server thread
-> frozen source -> validation -> PR/review -> delivery attempt -> integration/cleanup
```

TUI and Admin can render different densities, but both must resolve the same identifiers and
terminal result.

## Reject

### ACP As Akra's Core Runtime

The released Canvas Codex path is an Agent Server/ACP relay over embedded Codex crates. A later SDK
path replaces that Rust bridge with a Node ACP bridge that spawns an official app-server child.
Both create version and semantic translation boundaries that Akra does not need.

Reject:

- an ACP-to-Codex adapter between Akra and app-server;
- multi-provider parity as a reason to weaken Codex protocol fidelity;
- interpreting wrapper support as end-to-end support without tracing the Canvas consumer;
- copying plan, permission, fork, diff, identity, or reasoning semantics from the released ACP
  route.

Akra should instead improve its direct event projection and validate field preservation with a
golden trace.

### Browser IDE Duplication

Do not clone Monaco, a read-only xterm log, or a browser screenshot panel. Akra's primary surface is
the native inline terminal, and its operator needs live execution and delivery truth more than a
second editor shell. A future Admin detail view can show bounded diffs, logs, and provenance through
existing application services without becoming a full IDE.

### Generic Automation Marketplace

Do not build generic Slack, Notion, arbitrary SaaS actions, or an unconstrained script marketplace.
Akra automation exists to turn authenticated events into reviewed Codex code-change delivery. New
sources should map to that application command, not acquire independent workflow authority.

### Prompt-Only Delivery Controls

A button that asks an agent to pull, push, or open a PR is not evidence that the intended branch,
source SHA, checks, review, integration ref, or cleanup state is correct. Keep Git and GitHub
delivery in application services and adapters with explicit preconditions and persisted outcomes.

### Optional Isolation

Do not make worktree isolation a user-selected convenience for parallel delivery. The lane owns its
lease and worktree by contract. Shared-checkout conversational use can remain separate from
parallel-mode delivery authority.

### Browser-Stored Credentials And Automatic Approval

Remote node credentials must not enter browser local storage. ACP's first-option permission
selection must not influence Akra permission design. Operator approval, parent policy, and explicit
review-skipped delivery provenance are different authorities and must remain distinguishable.

## Differentiate

### Delivery, Not Conversation Completion

Canvas can show that an agent stopped and can link an automation run to a conversation. Akra should
prove a stronger terminal result:

```text
accepted intent
-> leased isolated worktree
-> source range frozen
-> validation bound to the exact frozen source SHA
-> source published
-> reviewed PR gates satisfied by default, or explicit high-risk exception recorded
-> integration applied in serialized authority
-> integration ref pushed and remotely verified
-> PR and source lane cleaned
```

Each transition needs an owner, time, source identity, reason, and retry/recovery state. Agent text
never substitutes for this evidence.

### Direct Codex Protocol Accountability

Canvas's ACP path shows why a protocol relay can advertise a feature while losing it before the
operator sees it. Akra's direct boundary should make that difference measurable:

- every official notification remains explicitly classified;
- retained events preserve native thread, turn, and item identity where the application needs it;
- bounded reductions declare what they omit;
- permission and error types do not collapse into generic text;
- a deterministic golden trace reports preserved, transformed, and dropped fields;
- protocol drift fails a contract test before it silently changes the UI.

This is an ongoing architectural invariant, not a request to expose every raw frame in the TUI.

### Truthful Game Operations

Akra's Admin diorama can become a differentiator only if it tells the truth. Current workers roam
and packets flow without a durable work transition, and the visual check expects idle pixels to
change. Replace that with semantic states and event-triggered motion:

- `idle`
- `starting`
- `working`
- `awaiting_review`
- `blocked`
- `delivering`
- `cleanup`
- `stale`

Movement and colored packets must be consequences of a new activity or lifecycle sequence. Ambient
motion, if retained, must be visually neutral and must not imply throughput. Unknown metrics remain
unknown rather than receiving fabricated progress.

### One Truth, Several Surfaces

Canvas benefits from a cohesive browser but delegates several core truths to independent services.
Akra should keep application services authoritative and expose the same state through native TUI,
Admin, CLI, Telegram, scheduler/webhook adapters, and later read-only remote nodes. Inbound handlers
must not mutate task, Git, GitHub, or SQLite state directly.

## Implementation Slices

### P0: Parallel Current Activity Envelope

This is the existing jcode parallel-activity work item with stronger priority and a more exact
contract, not a new parallel subsystem.

**Prerequisite**

The existing
[Protocol-Native Live Execution Rail](../jcode/gap-matrix.md#p0-protocol-native-live-execution-rail)
owns parsing and bounded reduction for item start, command delta, MCP/tool progress, and reasoning.
Until it lands, this slice can project only Akra's existing completed command/file activity and must
not label it `running`.

**Owned boundary**

- `src/domain/parallel_mode/agent_session.rs`
- parallel session-detail persistence and authority adapters
- `src/application/service/parallel_mode/{turn,orchestrator_loop,session_detail}`
- a fixed-capacity last-write-wins activity coalescer outside the bounded turn-stream receiver
- supervisor/control-plane projection
- existing TUI parallel roster/detail and Admin agent/task projection

**Contract**

- store `activity_kind`, `phase`, optional turn/item identity, bounded summary, UTC
  `last_activity_at`, lease generation, and authority-assigned runtime-event sequence;
- define a closed kind set for command, file change, MCP/tool, reasoning, and other/unknown without
  parsing presentation text;
- keep validation, review, delivery, cleanup, and blocked state in their existing lifecycle fields
  rather than disguising them as app-server activity kinds;
- accept an event only for the current lease generation and discard late stale-lane events;
- assign sequence only when the authority persists a relevant event; input stream events do not
  pretend to arrive with a global sequence;
- do not await one SQLite write per delta inside the capacity-eight turn receiver;
- retain at most 32 item keys per lease; replace progress for the same key, evict the oldest terminal
  key before an active key, and aggregate additional distinct active items into an explicit overflow
  count rather than growing memory;
- reserve eight of those coalescer entries for phase/terminal updates; progress cannot evict them;
- a terminal update for a retained item replaces its progress entry;
- terminal updates for unretained overflow items increment bounded outcome counters and the
  truncation marker rather than allocating another key or disappearing;
- when all retained/reserved capacity is exhausted, drop only replaceable progress, increment a
  persisted dropped-update counter, and surface truncation;
- persist at most one progress snapshot per second and flush terminal state outside the receiver;
- define in-process terminal retention separately from crash durability: turn completion performs a
  synchronous bounded final handoff, while a process crash can lose at most the last coalescing
  window and must recover as stale/unknown rather than claim a terminal state;
- retain lifecycle age separately from last app-server activity age;
- preserve existing validation, review, cleanup, conflict, and distributor history.

**User outcome**

After the protocol prerequisite lands, the operator can scan `cargo test running`, `patching
src/...`, or `quiet for 43s` alongside the separate lifecycle state such as `awaiting review`,
without opening each transcript. TUI and Admin drill into the same record.

**Required proof**

- command, file, MCP/tool, phase transition, truncation, and unknown-event reducer tests;
- stale lease generation, late item, and concurrent authority-sequence assignment tests;
- write-rate/coalescing, 32-key overflow, reserved-terminal saturation, dropped-progress accounting,
  synchronous final-handoff, crash-window, and bounded-receiver nonblocking tests;
- SQLite restart round trip;
- supervisor, TUI narrow/wide, and Admin JSON projection tests;
- desktop/mobile Admin capture and one real two-lane run.

### P0: Truthful Diorama State Machine

**Owned boundary**

- existing domain/application lifecycle and activity truth
- Admin inbound read-model mapping for `DioramaVisualState` and per-agent
  `visual_transition_id`
- `assets/admin/game/src/akra-diorama.ts`
- Admin dashboard integration and current visual-capture script

**Contract**

- derive one explicit visual state from typed application lifecycle/activity data;
- derive each agent's `visual_transition_id` from its lease generation and latest relevant persisted
  runtime-event sequence, never a global snapshot revision;
- seed the first snapshot as a non-animated baseline even when it already has a transition ID;
- trigger semantic movement and packets only when that agent's transition ID changes because of an
  allowlisted lifecycle/activity event;
- resolve simultaneous facts with typed precedence:
  `stale > blocked > awaiting_review > delivering > cleanup > working > starting > idle`;
- render every precedence state distinctly and keep the lower-priority facts available in detail;
- when polling skips several relevant sequences, reconcile once to the latest authoritative state
  and do not fabricate one packet per missing event;
- keep ambient animation neutral and separate from semantic work animation;
- never calculate state by parsing human summaries;
- respect reduced-motion without removing textual/state detail;
- keep unknown KPI values visibly uncollected.

**User outcome**

The game layer becomes an honest compressed operations view. Motion means a recorded transition;
stillness does not hide a blocker, and decorative motion does not claim work happened.

**Required proof**

- projection tests for every visual state and unknown data;
- first hydration with a nonempty transition ID produces no movement or packet;
- repeated identical snapshot leaves semantic positions and packet count unchanged;
- one relevant event produces exactly one agent transition, while unrelated/global events produce
  none;
- skipped-sequence catch-up produces one reconciliation transition without replayed packets;
- pairwise and representative multi-state precedence tests;
- blocked/idle/stale and reduced-motion tests;
- desktop/mobile canvas pixel checks plus keyboard-accessible detail;
- update the current idle-motion assertion so it proves stability rather than fabricated activity.

### Existing P0 Dependency: Native Performance Evidence Contract

Do not introduce a second benchmark schema. Use the jcode
[Native Performance Evidence Contract](../jcode/gap-matrix.md#p0-native-performance-evidence-contract)
after the active `fix/native-validation-evidence-contract` lane releases its capture/attestation
schema ownership. That active lane hardens validation artifacts; it does not implement the benchmark
driver, PSS accounting, or percentile contract. Performance remains a separate later slice.

Add a Canvas comparison profile only then:

- cold usable browser UI and warm conversation resume;
- submit to first event and first assistant delta;
- three-conversation list/detail propagation versus Akra's three-slot board propagation;
- idle and active complete-process-tree memory;
- explicit topology labels for browser, static proxy, Agent Server, Automation, ACP, and Codex;
- at least 30 attempted samples per comparable mode, raw failures retained, exact versions and auth
  state recorded.

Canvas release-gate durations and chunk sizes are not inputs to this comparison. No performance
winner is currently established.

### P1: Automation Intake Core

**Owned boundary**

- new domain trigger definition, intake key/disposition, run identity, and task-template types
- an `AutomationIntakeService`
- immutable `AutomationIntakeInvocation`, canonical `AutomationIntakeReceipt`, accepted-intent/outbox,
  trigger/run store port, and SQLite adapter
- intake-to-planning reconciliation service
- existing planning mutation service
- a JSON CLI inbound adapter for contract-first exercise

**Contract**

- define schedule and event triggers with workspace scope, timezone, enabled state, typed parameters,
  and an operator-owned task template;
- require a trigger-specific idempotency key before accepting a task: schedule uses
  `(trigger_id, scheduled_for_utc)`, webhook uses `(trigger_id, provider_event_id)`, and run-now uses
  `(trigger_id, operator_request_id)`;
- store invocation disposition as accepted, duplicate, disabled, rejected, or conflict, separately
  from run phase and delivery outcome;
- persist one immutable invocation for every distinct inbound request ID with source, actor,
  idempotency key, bounded parameter hash, disposition, reason, receive time, and optional canonical
  receipt reference;
- bind each canonical key claim to workspace, trigger, source, actor, normalized parameter hash, and
  task-template revision;
- exact replay of the same request ID and fingerprint returns the same invocation/result;
- a new request ID with an already-claimed key and the same fingerprint stores a duplicate invocation
  and returns `Duplicate { canonical_receipt_id }`, but creates no run or task;
- reuse of a request ID or idempotency key with a different bound fingerprint stores an immutable
  conflict invocation/result and creates no run or task;
- for the first invocation of an idempotency key, atomically persist the invocation and canonical
  receipt/key claim even when disabled or rejected, so later replay cannot become accepted after a
  policy/configuration change;
- when that canonical disposition is accepted, the same transaction also persists deterministic
  run/task IDs and a pending planning outbox intent before calling the planning boundary; disabled
  and rejected receipts have no run/task;
- make planning task creation idempotent by deterministic task ID, then CAS-bind the returned task
  and run to the pending intent; restart reconciliation completes an unbound intent or records a
  terminal binding failure without accepting a second task;
- never promote raw event payload text into system/developer instructions or shell commands;
- use application commands for every task mutation.

**User outcome**

An authenticated caller with an explicit idempotency key can request a bounded code change exactly
once, and the operator can prove whether intake was accepted, duplicated, disabled, rejected, or
conflicted after restart. This slice does not claim delivery success.

**Required proof**

- duplicate, out-of-order, and concurrent idempotency-key tests for schedule, webhook, and run-now
  key shapes;
- same-request replay, distinct-request duplicate invocation/audit, and same-key divergent payload,
  actor, template revision, and concurrent conflict tests;
- disabled trigger, invalid timezone, malformed parameter, and template-bound tests;
- disabled/rejected-then-reenabled replay tests proving the original key cannot create work later;
- crash/restart tests after receipt/intent commit, after task creation, and before/after run-task CAS
  binding, including two reconcilers racing the same intent;
- cross-workspace isolation and untrusted-payload tests;
- JSON CLI trace from request to intake disposition, run identity, and accepted task.

### P1: Schedule Intake Control Loop

Start only after Automation Intake Core is merged.

**Owned boundary**

- scheduler inbound control loop and injected clock
- durable due-trigger claim port/adapter
- `AutomationIntakeService` command boundary

**Contract**

- scheduler uses an explicit IANA timezone and a durable claim so a due run is accepted once;
- calculate `(trigger_id, scheduled_for_utc)` deterministically across restart and DST boundaries;
- require `misfire_policy` to be `skip` or `latest_once`, default `latest_once`; one scan creates at
  most one catch-up intent per trigger and records the skipped interval/count, never unbounded
  historical runs;
- call only the intake application service and never write planning or run rows directly;
- record skipped/disabled intake as a disposition rather than fabricating a failed delivery.

**User outcome**

An enabled schedule requests one bounded planning task for each on-time due instant. After downtime,
missed instants produce zero or one catch-up task according to the recorded `skip` or `latest_once`
policy, never an unbounded replay or duplicate.

**Required proof**

- timezone/DST gap and overlap tests;
- duplicate claim, two-process race, long-downtime `skip`/`latest_once`, bounded catch-up, disabled
  trigger, and clock-skew tests;
- one real schedule-to-accepted-task trace.

### P1: Signed Webhook Intake Adapter

Start only after Automation Intake Core is merged.

**Owned boundary**

- HMAC webhook inbound adapter
- source identity, timestamp, body-bound, and rate-limit policy
- `AutomationIntakeService` command boundary

**Contract**

- verify source identity, bounded body, timestamp window, signature, and provider event ID before
  constructing a typed intake request;
- derive `(trigger_id, provider_event_id)` and make replay idempotent across restart;
- treat payload fields as untrusted typed parameters, never prompt or shell authority;
- call only the intake application service and never mutate task/run storage from the handler;
- fail closed when source identity, time, signature, event ID, or ingress rate policy is unavailable.

**User outcome**

A signed provider event can request one bounded code-change task, while replay, malformed payload,
and unauthenticated traffic cannot create additional work.

**Required proof**

- signature, timestamp expiry, replay, body-size, rate, missing-event-ID, and malformed-parameter
  tests;
- restart and concurrent-delivery idempotency tests;
- cross-workspace and adversarial payload tests;
- one real signed webhook-to-accepted-task trace.

### P1: Automation Run Reconciliation And Provenance

This slice depends on the existing jcode
[Frozen-Source Validation Evidence](../jcode/gap-matrix.md#p0-frozen-source-validation-evidence),
[Active Turn Exit And Restart Recovery](../jcode/gap-matrix.md#p0-active-turn-exit-and-restart-recovery),
[Critical Review Response Loop](../jcode/gap-matrix.md#p1-critical-review-response-loop), and
[Authoritative Delivery Outcomes](../jcode/gap-matrix.md#p1-authoritative-delivery-outcomes) before it
can claim a terminal reviewed-delivery trace.

**Owned boundary**

- automation run/attempt phase and linkage domain
- reconciliation application service, versioned `CancelAutomationRun` command, and durable run store
- read ports for planning, parallel session, validation, PR, and delivery-attempt truth
- shared `CancelQueuedPlanningTask`, lease-bound `StopParallelLane`, and
  `AbandonDeliveryAttempt` application commands in their existing owning services

**Contract**

- model queued, active, cancelling, cancelled, skipped, timed-out, failed, finished, and unknown run
  phases separately from intake disposition;
- link attempts and recovery reasons without redefining the delivery outcome enum;
- reference the immutable authoritative delivery-attempt outcome when it exists;
- never infer `integrated` from conversation completion, agent text, PR presence, or a legacy
  validation summary;
- reconcile after restart with compare-and-set state and record every correction reason;
- require cancellation request ID, actor, expected run version, and CAS/idempotency semantics;
- for an unbound intent, cancel the outbox entry and leave no task, lease, or worktree;
- for an accepted but unleased task, use `CancelQueuedPlanningTask`, persist its authoritative
  terminal task result, and leave no lease or worktree;
- for an active lane, move the run to `cancelling`, use lease-bound `StopParallelLane`, terminally
  stop the worker, CAS the owning planning task to its terminal cancelled state before lease release,
  and prevent redispatch; if that CAS fails, retain the lease/worktree and remain visibly blocked;
- abandon a delivery attempt only when one exists, never synthesize an attempt or outcome for an
  active lane that has not reached delivery;
- keep the lease/worktree owned until normal cleanup confirms a clean released slot; only then mark
  the run cancelled;
- after source publication but before integration starts, use the terminal-abandon command from
  Authoritative Delivery Outcomes, retain PR/audit history, apply the normal branch/worktree cleanup
  policy, and confirm cancellation only after the terminal attempt is persisted;
- return `too_late` without mutation after integration application starts or a remote integration is
  already verified;
- record requested, confirmed, failed, or too-late cancellation separately and never overwrite an
  immutable authoritative delivery outcome;
- never edit task, slot, SQLite, Git, worktree, or GitHub state directly from the cancellation
  command.

**User outcome**

The operator can follow one accepted automation run through task, session, exact-source validation,
review, delivery attempt, and cleanup without confusing run phase with delivery success.

**Required proof**

- phase/outcome separation, retry-attempt, cancellation race, timeout, unknown-state, and restart
  reconciliation tests;
- cancellation tests for unbound intent, queued task, active turn, source-published delivery,
  integration-started, already-integrated, stale version, duplicate request, and crash recovery;
- controlled-stop and cleanup failure tests proving the lease/worktree remain owned and the run does
  not become cancelled prematurely;
- active cancellation with and without a delivery attempt, planning-task CAS failure, and
  post-cancellation no-redispatch tests;
- tests proving conversation completion and PR open do not imply integration;
- provenance round trip to immutable delivery outcome and cleanup;
- one real run trace ending in remotely verified integration or authoritative terminal failure.

### P1: Admin Automation Run Trace And Guarded Controls

Start after run reconciliation and reuse the jcode
[Guarded Admin Control Actions](../jcode/gap-matrix.md#p1-guarded-admin-control-actions)
authentication, CSRF, version/CAS, idempotency, confirmation, and actor audit contract.

**Owned boundary**

- shared automation run read projection
- authenticated Admin list/detail/run-now/cancel handlers and templates
- a guarded `RunAutomationNow` wrapper over `AutomationIntakeService` and the versioned
  `CancelAutomationRun` command

**Contract**

- show trigger, intake disposition, attempts, queue age, current activity, recovery reason, and deep
  links to task, session, exact-source validation, PR/review, delivery outcome, and cleanup;
- render every known and unknown phase safely;
- give run-now a fresh operator request ID and implement run-now/cancel only through those guarded
  application commands;
- never mutate task, run, SQLite, Git, worktree, or GitHub state directly from HTTP handlers.

**User outcome**

Operators can inspect the whole reviewed-delivery trace and issue audited run-now or cancellation
commands without turning Admin into an independent automation authority.

**Required proof**

- exhaustive/unknown status rendering and link-integrity tests;
- authorization, CSRF, stale-version, idempotency, actor-audit, and cancellation-race tests;
- Admin JSON/template tests and desktop/mobile Playwright captures;
- one real audited run-now or cancel trail.

### P2: Read-Only Node Snapshot Protocol And Server

**Owned boundary**

- versioned node identity/capability/snapshot wire DTOs
- application query service over existing task/session/delivery projections
- dedicated read-only HTTPS listener with Rustls mutual TLS, disabled by default
- response minimization and size-bound policy

**Contract**

- serve Akra node identity, protocol/product version, capabilities, workspace-scoped bounded snapshot,
  and server observation time;
- require an operator-provisioned server certificate/key and client-CA bundle; bind node identity to
  a URI SAN on the server certificate;
- make an operator-owned ACL keyed by the client certificate's SHA-256 fingerprint the sole
  authorization source for `(client_id, workspace_set)`, with at most 64 enrolled clients;
- require a timestamp within 60 seconds and unique request ID; retain up to 256 IDs per client
  fingerprint for two minutes, reject duplicates, and reject new requests when that client's cache
  is full rather than evict an unexpired ID;
- support trust-bundle reload with overlapping old/new client CAs for rotation and an explicit
  revoked-certificate fingerprint list;
- keep this listener separate from the loopback Admin listener;
- minimize task/session/delivery data to fields required for oversight and exclude prompt bodies,
  secrets, raw tool output, credentials, and arbitrary filesystem paths;
- reject unknown versions, unauthorized workspaces, expired/revoked certificates, replayed/stale
  request IDs, and oversized
  requests before projection;
- expose no remote mutation method.

**User outcome**

A trusted remote client can read one bounded, workspace-scoped Akra operations snapshot without
opening the local Admin UI or gaining mutation authority.

**Required proof**

- protocol-version, capability, bound, and deterministic serialization tests;
- mutual-TLS identity, expiry, revocation, CA rotation, replay/stale request ID, and cross-workspace
  failure tests;
- per-client request-ID flood and cache-full fail-closed tests with no unexpired eviction;
- response fixtures proving minimized fields and credential/prompt/output non-disclosure;
- one real remote read through the selected transport.

### P2: Private Node Registry And Read Client

Start after the node snapshot protocol is merged.

**Owned boundary**

- remote node registry domain and port
- OS-private/server-side credential and trust-store adapter
- enrollment, certificate rotation, and revocation service
- bounded mutual-TLS HTTPS snapshot client

**Contract**

- register Akra nodes, not model providers;
- enroll an endpoint DNS name, expected node identity, server CA, client certificate/key reference,
  supported version, and allowed workspace scope;
- verify HTTPS peer identity and reject redirects, DNS/SAN mismatch, certificate mismatch or expiry,
  revoked identity, unsupported version, oversized response, and cross-workspace data;
- rotate server/client trust with an explicit overlap deadline and revoke by full SHA-256 certificate
  fingerprint without deleting audit history;
- retain health, last success/failure, remote observation time, local receipt time, and explicit
  snapshot age;
- keep credentials outside browser state and redact them from logs, telemetry, API responses, and
  error messages;
- perform reads only.

**User outcome**

Akra can enroll, rotate, revoke, and health-check remote Akra nodes without storing trust material in
the browser or adding a provider/runtime relay.

**Required proof**

- two-node enrollment, read, rotation, revocation, and recovery tests;
- certificate/host/version/redirect/size/workspace failure tests;
- stale, timeout, clock-skew, and bounded-cache tests;
- credential non-disclosure checks across storage, logs, telemetry, and API output.

### P2: Remote Node TUI And Admin Projection

Start after the protocol and private registry are merged.

**Owned boundary**

- shared node-health and snapshot-age application projection
- TUI node selector/detail
- Admin node list/detail using server-side data only

**Contract**

- show node identity, version, capabilities, workspace, health, last success/failure, and snapshot
  age;
- label stale/cached state without presenting it as current;
- preserve stable links to the remote task/session/delivery identities exposed by the bounded
  protocol;
- never serialize node credentials to HTML, JSON, JavaScript, or browser local storage;
- expose no remote command in this slice.

**User outcome**

An operator can see which node owns a session or delivery and whether that view is healthy or stale
from TUI or Admin, while all trust material remains server-side.

**Required proof**

- two-node selection, stale/timeout/recovery, and identity-link tests;
- narrow/wide TUI snapshots and desktop/mobile Admin captures;
- browser storage, HTML, JSON, log, and telemetry credential canaries.

Remote commands require a later protocol, actor/audit model, and separate review. They are not part
of these slices.

## Comparative Experiments

These experiments validate differentiation but do not block the first two P0 product corrections.

### Golden Protocol Fidelity Trace

Drive deterministic fake app-server scenarios with a shared prefix containing text phases,
reasoning, plan, turn diff, patch delta, command output delta, permission, MCP elicitation, image,
and usage breakdown. Split mutually exclusive terminal paths into at least normal completion,
generic stream/error, auth/quota failure, and interrupt scenarios. Split permission and elicitation
into accept, reject, and cancel branches. Feed the same scenarios through Akra and the later
maintained ACP path where `CODEX_PATH` permits it. Record canonical JSON at app-server, ACP, SDK,
Canvas consumer, and Akra reducer boundaries. Mark every field preserved, transformed, or dropped,
including order and correlation identity.

The released embedded bridge cannot accept the same fake app-server. Treat its checked fixtures as
a separate stratum rather than pretending the paths are identical.

### Permission And Secret Canary

Generate command, file, network, and MCP requests with varied advertised decision sets, including
allow-once, allow-session, reject, and cancel where the protocol supports them. Enumerate what each
layer actually advertises and preserve unsupported or collapsed choices as results rather than
assuming parity. Vary outer OpenHands confirmation off/on, ACP session mode, and permission option
order as separate dimensions. Record whether the request reaches an operator UI and the exact
selected option ID. For Akra, explicitly record its current accept/decline reduction, rejection of
accept-for-session-only command requests, automatic decline of uninspectable file-change approval,
and automatic MCP elicitation decline.
Place unique canaries in parent environment, registered environment secrets, file secrets, tool
child environment, and stored events. Run concurrent same-provider conversations with ambient login
and `CODEX_AUTH_JSON` as separate strata; record provider data-directory paths/inodes and provoke
auth refresh, config, and lock activity. For the later bridge, test `APP_SERVER_LOGS` off/on with
separate startup auth/config, prompt, and app-server frame canaries. Compare with Akra API-key
forwarding off and on.

### Crash, Resume, And Drift Matrix

For the released topology, kill Agent Server or the embedded-core Zed bridge at initialization,
turn acceptance, first delta, active tool, pending approval, and completion-before-client
acknowledgement. For the later topology, independently kill Agent Server, the maintained Node
bridge, and app-server at the same boundaries. Record orphan processes, duplicate turns, retained
IDs, history hashes, visible notices, and fresh-session fallbacks. Ambiguous active turns must not be
automatically resubmitted. Invoke SDK `ask_agent()` directly against both bridges and record
capability gating, error code, and child/session side effects.

Test Python ACP 0.10.1 and 0.11.x against pinned and next wrappers. Treat 0.11.x as a negative test
outside Canvas's released constraint. Changing the released embedded Codex patch requires rebuilding
the Rust bridge; only the later bridge can swap a Codex executable. Inject an unknown app-server
notification into Akra and the later bridge, then inject an unknown ACP `session/update` and wire
message at the Python/released boundary. Verify that Akra's classification test fails and record how
each applicable boundary behaves.

## Recommended Order

This order covers Canvas-derived work only and does not supersede the jcode
[global order](../jcode/gap-matrix.md#recommended-order).

1. In the existing disjoint protocol/TUI lane, land Protocol-Native Live Execution Rail before
   claiming live parallel activity; then implement Parallel Current Activity Envelope in the
   existing parallel-activity ownership lane.
2. Make the Admin diorama truthful after the typed activity/lifecycle projection is available.
3. After the active validation-schema lane releases ownership, start the separate Native
   Performance Evidence Contract and later add the Canvas profile without changing its schema.
4. Automation Intake Core can proceed in a disjoint domain/application lane; add schedule and signed
   webhook intake as separate PRs after the core.
5. Complete Frozen-Source Validation Evidence, Critical Review Response Loop, and Authoritative
   Delivery Outcomes in the jcode order before Automation Run Reconciliation can claim reviewed
   delivery; add the Admin run trace only after reconciliation and Guarded Admin Control Actions.
6. Run the golden protocol, permission/secret, and crash/resume experiments before making public ACP
   fidelity or performance claims.
7. Add remote support in protocol/server, private registry/client, and TUI/Admin projection PRs only
   after local delivery and automation outcomes are authoritative.

## Success Audit

This comparison has produced value only when later evidence proves:

- every active parallel lane exposes bounded current activity, last-activity age, lease generation,
  validation source, review/delivery state, and cleanup truth;
- Admin motion and packets correspond to recorded sequence transitions and idle snapshots remain
  semantically stable;
- an event is accepted at most once and cannot turn raw payload text into agent authority;
- trigger-specific intake disposition survives restart and schedule, webhook, and run-now keys do
  not collide;
- run state survives restart, links trigger, task, session, exact-source validation, PR, delivery
  attempt, and cleanup, and references rather than redefines the authoritative outcome;
- cancelled, skipped, timed-out, failed, unknown, and successful states render exhaustively;
- the remote snapshot server minimizes data, remote credentials never enter browser storage, and
  every first-generation node protocol/client/UI method is read-only;
- a versioned artifact, not topology intuition, supports any performance or fidelity comparison;
- no ACP runtime, provider registry, browser IDE clone, or generic automation marketplace displaced
  the reviewed Codex delivery goal.
