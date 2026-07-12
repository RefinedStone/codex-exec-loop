# OpenCode To Akra Gap Matrix

This matrix turns the [v1.17.18 analysis](analysis.md) and
[evidence ledger](evidence.md) into Akra-relative decisions. `Ahead` means stronger evidence for the
named dimension at the pinned snapshots, not a total product score. Accepted work must strengthen
Akra's Codex-first operating and reviewed-delivery position rather than copy OpenCode's platform
breadth.

## Relative Matrix

| Capability | OpenCode v1.17.18 | Akra `354f4782` | Decision | Priority | Evidence |
| --- | --- | --- | --- | --- | --- |
| Runtime authority | owns providers, inference, tools, sessions, plugins, server, and clients | delegates model/tool runtime to official app-server and owns operator/delivery flow | reject runtime duplication | invariant | [OpenCode](evidence.md#product-release-and-topology), [Akra](evidence.md#akra-baseline-evidence) |
| Client detach | separate `serve` plus `attach` can decouple client lifetime; default TUI stops its worker; live detach not reproduced | child connection owns turn runtime; transcript can resume | extend existing recovery contract without claiming survival | existing P0 | [continuity](evidence.md#session-continuity-forks-and-background-work), [Akra](evidence.md#akra-baseline-evidence) |
| External reconnect | retries SSE without replay cursor or complete TUI resync | child restart reconciliation is incomplete | reconcile protocol-guaranteed fields and label omitted history | existing P0 | [events](evidence.md#product-release-and-topology), [existing slice](../jcode/gap-matrix.md#p0-active-turn-exit-and-restart-recovery) |
| TUI command discovery | command palette, leader bindings, which-key groups, contextual session actions | one searchable `:` registry and palette; availability/reason metadata is absent | extend the existing registry after core protocol work | P2 | [TUI](evidence.md#tui-commands-and-long-sessions), [Akra](evidence.md#akra-baseline-evidence) |
| Live execution | cohesive command, tool, diff, permission, question, timeline, and child views | bounded completed-item summaries; richer native events are planned | reuse existing live rail | existing P0 | [TUI](evidence.md#tui-commands-and-long-sessions), [existing slice](../jcode/gap-matrix.md#p0-protocol-native-live-execution-rail) |
| Long sessions | TUI hydrates 100 recent messages; browser pages older history | official session list/search with local paging; full protocol provenance planned | adaptive projection plus explicit truncation | existing P0/P2 | [history](evidence.md#tui-commands-and-long-sessions), [existing session slice](../jcode/gap-matrix.md#p2-official-session-search-and-provenance) |
| Approval choices | typed permission/question UX and richer action choices | binary accept/decline; uninspectable file/MCP requests decline | preserve command/permission choices first; forms remain fail closed | P1/P2 | [OpenCode UI](evidence.md#tui-commands-and-long-sessions), [Akra](evidence.md#akra-baseline-evidence) |
| Forking | transcript-copy fork without explicit source lineage | official schema has `thread/fork` and `forkedFromId`, but no Akra path | add as a bounded sub-slice of official session provenance | existing P2 | [fork](evidence.md#session-continuity-forks-and-background-work), [Akra schema](evidence.md#akra-baseline-evidence) |
| Background agents | child sessions plus experimental process-local jobs in shared directory | durable fixed lanes with worktree isolation and delivery detail | adopt lineage UX; reject shared-directory authority | existing P0/P1 | [background](evidence.md#session-continuity-forks-and-background-work), [Akra delivery](evidence.md#akra-baseline-evidence) |
| Worktree UX | create/reset/remove/startup commands; reset is destructive | leased lane identity and guarded cleanup | different; keep Akra lane contract | shipped | [worktrees](evidence.md#worktrees-github-and-delivery), [Akra](evidence.md#akra-baseline-evidence) |
| GitHub intake | issue/PR/comment/schedule/dispatch events | no shipped schedule/webhook intake | reuse Canvas automation plan | existing P1 | [GitHub](evidence.md#worktrees-github-and-delivery), [automation slices](../agent-canvas/gap-matrix.md#p1-automation-intake-core) |
| GitHub delivery | prompt-driven commit/push/PR; mutable action; no verified reviewed integration authority | frozen source, review/check default gates, serialized integration, remote verification, cleanup | differentiate and add provenance deep links | shipped plus existing P0/P1 | [OpenCode](evidence.md#worktrees-github-and-delivery), [Akra](evidence.md#akra-baseline-evidence) |
| Server authentication | password optional; default loopback; reproduced raw secret responses without password | loopback-only Admin with capability/session auth and origin guard | preserve fail-closed boundary; add negative proof to the release invariant | invariant | [canary](evidence.md#resolved-secret-canary), [Akra](evidence.md#akra-baseline-evidence) |
| Secret handling | resolved config/provider values exposed; full plugin/MCP trust; repo Git config token | child env scrubbed by default with explicit full-env override; bounded egress not fully proven | extend the existing canary experiment as a release invariant | invariant | [security](evidence.md#authentication-secrets-permissions-and-extensions), [Akra](evidence.md#akra-baseline-evidence) |
| Permission composition | direct `.env` read asks but in-workspace shell path does not compose that rule | upstream Codex owns tools; Akra owns child env and app-server approval projection | verify official capability with canaries; do not build a second tool broker | existing experiment/invariant | [OpenCode](evidence.md#authentication-secrets-permissions-and-extensions), [existing canary](../agent-canvas/gap-matrix.md#permission-and-secret-canary) |
| Browser/desktop | shared full product app and six desktop targets | authenticated operational Admin, no desktop | reject surface race; strengthen operator projection | invariant | [surface](evidence.md#web-desktop-ide-and-sharing), [Akra](evidence.md#akra-baseline-evidence) |
| Remote nodes | generic server connection and optional credentials | local Admin plus Telegram; read-only node plan exists | keep existing read-only plan | existing P2 | [server](evidence.md#product-release-and-topology), [node plan](../agent-canvas/gap-matrix.md#p2-read-only-node-snapshot-protocol-and-server) |
| Sharing | manual/auto upload; GitHub Action defaults to sharing public-repo sessions | no equivalent public share | reject until a separate redacted export use case exists | reject | [share](evidence.md#web-desktop-ide-and-sharing) |
| Performance evidence | manual browser renderer suite without portable budgets or packaged Electron coverage | no versioned complete process-tree baseline | winner unknown; reuse one evidence contract | existing P0 | [limits](evidence.md#performance-and-quality), [existing slice](../jcode/gap-matrix.md#p0-native-performance-evidence-contract) |
| Release quality | green Turbo core/app tasks plus separately run workflow-ungated desktop tests; desktop/i18n/publish wiring gaps | broad Rust/native tests; exact-SHA validation evidence remains planned | green evidence must bind to released source | existing P0 | [quality](evidence.md#performance-and-quality), [validation slice](../jcode/gap-matrix.md#p0-frozen-source-validation-evidence) |
| Game operations | no comparable product surface | differentiated Admin diorama, but semantic truth work is planned | keep existing truthful state-machine slice | existing P0 | [Canvas slice](../agent-canvas/gap-matrix.md#p0-truthful-diorama-state-machine) |

## Portfolio Rules

OpenCode supplies new evidence for several already-owned Akra work items. It must not create
parallel backlog names for them.

| OpenCode lesson | Owning existing Akra item | Amendment only |
| --- | --- | --- |
| separate server-scoped async prompt and attach | [Active Turn Exit And Restart Recovery](../jcode/gap-matrix.md#p0-active-turn-exit-and-restart-recovery) | add external transport loss, field-level authority/omission mapping, event-gap/stale-state proof, and explicit default-TUI/detach/crash labels |
| diff/timeline/100-message hydration | [Protocol-Native Live Execution Rail](../jcode/gap-matrix.md#p0-protocol-native-live-execution-rail) | add adaptive long-session windows, truncation markers, and official older-history drilldown |
| browser session paging and child navigation | [Official Session Search And Provenance](../jcode/gap-matrix.md#p2-official-session-search-and-provenance) | include fork/child lineage and stable deep links; do not add embeddings |
| GitHub events and schedule UX | [Automation Intake Core](../agent-canvas/gap-matrix.md#p1-automation-intake-core), then schedule/webhook slices | include trigger-to-session/branch/PR deep links; retain typed templates and idempotency |
| renderer throughput and frame-gap metrics | [Native Performance Evidence Contract](../jcode/gap-matrix.md#p0-native-performance-evidence-contract) | add dense diff, long transcript, streaming burst, and resize profiles under the same raw artifact schema |
| agent completion versus delivery | [Authoritative Delivery Outcomes](../jcode/gap-matrix.md#p1-authoritative-delivery-outcomes) | retain separate agent/session, source, review, integration, and cleanup outcomes |

OpenCode does not create a new P0. Its detach, live rail, performance, validation, activity,
automation, and session lessons amend the existing jcode/Canvas portfolio. New reviewable work is
limited to a narrow P1 approval-choice slice and three P2 follow-ups: inspectable file approval,
typed request forms, and contextual metadata for Akra's existing command registry. Official fork
lineage is a bounded sub-slice of the existing P2 session provenance contract. The canary work
remains a comparative experiment and release invariant whose matrix grows with each owning feature.

## Adopt

### 1. Exact Choice Fidelity

OpenCode shows the operator value of typed permission and question forms. Akra's pinned official
schema is richer still: command approval can offer one-shot accept, session accept, exec-policy
amendment, network-policy amendment, decline, or cancel; server requests include typed file,
permission, input, and MCP elicitation forms.

Adopt the exact app-server request method and its advertised `availableDecisions`. Do not invent a
universal approve button. One-shot acceptance remains the default. Persistent or session-scoped
choices appear only when the server offered them, after a second explicit operator action, with the
scope and proposed rule visible. Unattended workers continue to decline.

### 2. Official Fork Lineage

OpenCode's fork UX is useful, but transcript copying without explicit lineage is weaker than the
official Codex primitive already present in Akra's schema. Adopt a fork action from the session
browser and current session, optional last-turn selection, and a stable link back to the source
thread. Store only the application provenance needed to associate the fork with accepted task and
delivery truth; app-server remains thread authority.

### 3. Contextual Command Discovery

Akra already has one registry and searchable palette. Add fields, not another command system:

- stable command identity, label, aliases, argument shape, and grouping;
- current availability and a bounded reason when disabled;
- discovery-only leader/which-key bindings that call the same registry entry;
- palette ranking based on typed current mode and recent explicit use, not hidden telemetry;
- width-aware projection and a complete `:help` fallback.

Approval resolution, text editing, and destructive confirmations remain modal and cannot be
triggered by a stale palette entry.

### 4. Reconciled External State

OpenCode's browser repairs more root, directory, catalog, and status state than its external TUI,
but it does not prove full cached-transcript reload. Akra should go further where official APIs
permit: invalidate stale selected windows after a possible gap, reconcile from official thread/turn
truth and Akra's durable planning/delivery state, and label any remaining ambiguity. Akra cannot
make app-server emit an application-owned replay cursor, and silence never proves completion.

### 5. Cohesive Drilldown

OpenCode places diff, timeline, permission, question, and child navigation around the selected
session. Akra should apply the information architecture to its own facts: a bounded live rail,
selected full diff, current approval/elicitation, official fork lineage, and delivery provenance.
Do not copy a generic file browser, PTY, or browser IDE.

## Reject

### Provider And Tool Runtime Duplication

Reject multi-provider routing, local model/tool ownership, a parallel permission engine, and ACP as
Akra's core runtime. They weaken the reason to choose a Codex wrapper and create a second authority
for semantics the official app-server already owns.

### Optional Or Raw Remote Control

Reject an authentication-optional server, password-bearing browser local storage, and API routes
that expose or mutate raw config, providers, credentials, files, PTYs, shell, or runtime plugins.
The planned remote-node surface remains read-only, mutually authenticated, versioned, and derived
from application DTOs.

Reject public-repository default session sharing. A future explicit export must enumerate and redact
its payload, show the destination and policy, require an operator action, and remain separate from
delivery success.

### Arbitrary Plugins And Mutable Installers

Reject in-process arbitrary plugins, automatic LSP installation from mutable default-branch
archives, `@latest` delivery actions, and full-environment MCP inheritance as Akra-owned extension
patterns. Official Codex chooses its own MCP/tool boundaries; Akra filters only the child environment
and application surfaces it owns.

### Same-Checkout Background Work

Reject conversational fan-out as parallel delivery authority. A background task that shares the
current checkout, loses ownership on restart, or reports only through a synthetic prompt cannot
replace Akra's task, lease, worktree, source, review, integration, and cleanup chain.

### Prompt-Only GitHub Delivery

Reject `git add .`, agent-managed branch bypass, and commit/push/PR completion as terminal delivery
proof. Keep Git and GitHub side effects in application services and adapters with frozen identity,
CAS, gates, remote verification, and recovery.

### Surface Breadth As Strategy

Reject a browser IDE, general desktop client, VS Code launcher, public transcript sharing, and a
generic remote harness until the native TUI and reviewed-delivery loop are measured and clearly
better. Admin remains an operations surface, not a second editor.

## Differentiate

### Credential-Bounded Authenticated Control Plane

Akra already has the stronger bootstrap: loopback-only bind, random `.localhost` origin, mandatory
capability/session authentication, app-server child environment scrubbed by default, and identity-
checked GitHub writes. Exact `AKRA_APP_SERVER_PROCESS_ENVIRONMENT=all` is an explicit elevated-risk
override and must remain visibly distinct in canary evidence. Turn the default structure into
executable product evidence. Every public DTO should be allowlisted, each credential/content class
should have declared positive and forbidden sinks, negative routes should be canary-tested, and
unknown fields should fail closed rather than serialize automatically.

### Codex Protocol Accountability

Akra should expose official command/file/permission/MCP choices, fork lineage, turn steering, diff,
and thread identity without a provider-neutral translation layer. The direct boundary is valuable
only if a golden trace proves which fields are preserved, reduced, rejected, or unsupported.

### Reviewed Outcome, Not Agent Completion

OpenCode can make a PR after an agent run. Akra should prove the stronger path:

```text
accepted intent
-> isolated lease/worktree
-> official session and fork lineage
-> exact frozen source and validation
-> source branch and PR
-> default review/check/mergeability gates, or explicit high-risk exception
-> serialized integration and remote verification
-> PR/source/slot cleanup
```

Every transition retains identity, time, reason, retry/recovery state, and deep links. Agent text is
never terminal proof.

### Truthful Native Operations

OpenCode has broader session UI; Akra can be better for a Codex delivery fleet. The TUI and Admin
should show the same live item, approval, fork, source, review, integration, and cleanup truth. The
game layer may animate only persisted semantic transitions. Native performance becomes a claim only
after the versioned process-tree contract is green. Any accepted asynchronous work/result must enter
a durable mailbox or queue with a wake generation and become either drained or explicitly pending;
a process-local synthetic prompt cannot be task or delivery authority.

## Release Invariant

### Known-Canary Bounded Egress Matrix

This turns the existing Canvas
[Permission And Secret Canary](../agent-canvas/gap-matrix.md#permission-and-secret-canary) experiment
as a release invariant for currently shipped Akra-owned boundaries. It is quality evidence, not a
new product P0 or an alternate Codex tool sandbox.

**Owned boundary**

- public DTOs in current application services and inbound Admin/CLI/Telegram adapters;
- app-server child environment filtering and redacted trace/log helpers;
- Git/GitHub credential injection and command diagnostics;
- current SQLite planning, session-detail, and delivery persistence adapters;
- test fixtures and a native validation script that scans captured outputs and repository Git state.

**Contract**

- inventory current Akra-owned app-server API-key forwarding, child environment, GitHub credential
  transport, Admin capability/session, Telegram bot credential, public allowlisted-user identifier,
  proxy URL credential, prompt-like untrusted marker, and private workspace content boundaries;
- record provider/MCP internal credentials only as upstream observations. Akra does not own their raw
  storage or transport, and the matrix must not claim to enforce an upstream sink;
- generate a unique synthetic canary for every applicable credential/content class. Public
  identifiers and control-injection markers are classified separately from secrets;
- define a source-to-sink matrix before injection. Credential canaries may reach only their named
  raw sink, such as the opted-in app-server child environment or ephemeral outbound HTTPS
  authentication transport. Prompt/file canaries may appear in the explicitly authorized
  disposable official transcript when the experiment requests that content; no real credential is
  used;
- treat handles, redacted descriptors, approved raw sinks, and forbidden secondary sinks as distinct
  expectations. Never claim that Akra can prevent official Codex from returning content the
  operator explicitly authorized it to read;
- define public DTOs by allowlist and never serialize raw provider/config/server-request structures;
- for credential canaries, prove absence from every TUI projection, Admin JSON/HTML, CLI JSON/text,
  Telegram payload, current SQLite text/blob column, filesystem planning artifact, log/trace/crash
  report, prompt, Git command argument/diagnostic, PR title/body/comment, and repository/worktree Git
  configuration outside the declared raw sink;
- for prompt/file canaries, allow the explicit disposable source transcript only, then prove Akra
  does not amplify the value into unrelated Admin summaries, Telegram messages, planning or delivery
  persistence, logs, Git metadata, or GitHub content without a separate explicit user action;
- prove that Admin requests without valid capability/session authentication cannot reach any data
  route and that noninteractive startup without an explicit token fails;
- treat the current interactive Admin bootstrap stream as the sole approved raw sink for a generated
  capability token, or remove that output in a separately reviewed behavior change; its normal
  appearance there is not a leak-test failure;
- preserve current exact opt-in API-key forwarding to app-server, but do not log the value or claim
  the upstream process cannot use it;
- capture default-scrubbed and exact `AKRA_APP_SERVER_PROCESS_ENVIRONMENT=all` cases separately;
  the latter is an elevated-risk override and cannot inherit the default boundary's green result;
- observe official Codex read/shell behavior with a disposable workspace canary and record each
  unsupported live path as `unknown`; this experiment does not gate current Akra-owned sinks and
  does not add an Akra tool-policy layer;
- scan bounded output artifacts as bytes as well as UTF-8 text, including only the URL/basic-auth/
  base64 representations the tested Akra boundary is known to produce; this is a bounded regression
  scanner, not general data-loss prevention;
- fail the matrix check on missing expected captures, scanner errors, unknown public DTO fields, or any
  forbidden match; never report green from an empty artifact directory;
- delete canary state after the run and verify source/worktree Git config and process environment
  contain no residue;
- after every disposable DB writer shuts down, scan the main SQLite file including free pages and
  bounded `-journal`, `-wal`, `-shm`, temporary, and backup candidates as raw bytes without opening
  or mutating them. Active-row queries and physical-file scans are separate evidence; a write/delete
  cycle cannot pass from an empty final query alone;
- authorize repository/actor identity before credential mint, untrusted attachment download, or
  other remote side effects;
- do not store raw third-party credentials in Akra application persistence. A future adapter that
  needs durable lookup must define its own reviewed secret-store boundary and add its matrix rows in
  that feature PR;
- inspect Git state through `git rev-parse --git-common-dir`, `git rev-parse --git-path config`,
  worktree config, includes, `GIT_CONFIG_*`, helpers, and askpass rather than assuming `.git` is a
  directory in every worktree. The ephemeral helper input or outbound HTTPS auth header is an
  approved raw sink; repository config is not.

**Evidence growth**

The initial scanner/manifest is shared quality infrastructure and covers only shipped boundaries.
Each later approval, automation, webhook, remote-node, MCP, or credential-bearing feature adds its
own source/sink rows and captures in that feature's reviewable PR. Existing rows can be green while
future or unsupported rows remain absent or explicitly `unknown`; missing required current captures
still fail closed.

**User outcome**

An operator receives exact-source evidence that Akra does not copy known credential values beyond
their approved transport sink or amplify explicitly authorized file/prompt content into unrelated
surfaces. The claim is bounded to the canary matrix; it is not a universal secret detector.

**Required proof**

- unit tests for DTO allowlists, recursive redaction, encoded variants, and unknown-field failure;
- integration tests for every named current inbound/outbound surface and SQLite/filesystem round trip,
  including positive assertions that each approved raw sink actually received its canary;
- disposable SQLite active-row plus post-writer physical main/sidecar/temp/backup scans, including a
  fixture that writes then deletes a canary and proves the raw scanner still detects residue;
- unauthorized Admin route matrix plus session idle/absolute-expiry tests;
- fake Git credential helper/askpass capture proving the approved sink received its canary while no
  command argument, common/worktree config, include, diagnostic, or repository file retained it;
- disposable official app-server canary trace for direct read, shell read, MCP, approval, interrupt,
  and timeout paths with explicit supported/unsupported results;
- process-tree teardown and residue scan;
- a tracked manifest containing source SHA, environment stamp, canary classes, expected captures,
  scanner version, and zero forbidden matches without storing the canary values.

## Implementation Slices

### P1: Command And Permission Choice Fidelity

This closes the first choice-fidelity gap identified in both Canvas and OpenCode. It is deliberately
limited to command and bounded permission approvals. File change, user input, and MCP elicitation
retain their current fail-closed behavior until separate P2 slices land.

**Prerequisites**

- Active Turn Steering and Active Turn Exit And Restart Recovery own child/turn loss and uncertain
  transport recovery;
- the current-surface known-canary matrix and protocol classification tests are green.

**Owned boundary**

- `src/domain/conversation.rs` typed request, decision, resolution, and provenance types;
- app-server approval/server-request parsing and response serialization;
- interactive runtime control port and conversation service;
- TUI modal state, input controller, and command/permission approval rendering.

**Contract**

- require server request ID, method, `threadId`, `turnId`, and `itemId` for command/permission
  ownership; only the distinct `approvalId` callback may be absent. Also retain child generation,
  workspace ownership, request age, decision-source classification, and bounded inspectable facts;
- model command decisions separately: one-shot accept, session accept, exec-policy amendment,
  network-policy amendment, decline-and-continue, and cancel/interrupt;
- classify `availableDecisions` before rendering: a non-empty valid array exposes exactly its offered
  variants; missing/null uses only the pinned legacy one-shot accept/decline set after every required
  identity/detail validates; empty, unknown, mixed-unknown, or malformed input receives a method-
  specific decline without reaching the UI;
- missing or mismatched required thread/turn/item ownership never reaches the UI and settles through
  one method-specific decline attempt;
- require a second confirmation for a session-scoped or persistent amendment, displaying exact
  scope, rule, host/protocol, and whether the choice persists beyond one command;
- render bounded permission profiles without losing network/filesystem detail;
- keep file-change, user-input, and MCP elicitation on their existing explicit decline responses;
- main interactive TUI may decide; parallel, planning, automation, Telegram, CLI, and disconnected
  runtimes decline unless a future explicit authority contract says otherwise;
- key local ownership by child generation, method, server request, workspace, thread, turn, and
  callback/item identity; a compare-and-set permits at most one local response write;
- represent `pending -> locally_decided -> write_attempted -> write_confirmed` as local transport
  facts. The pinned protocol provides no general remote-resolution acknowledgement, so remote state
  remains unknown unless a later method supplies explicit evidence. Disconnect or timeout after a
  write never becomes a fabricated accepted/declined terminal result;
- timeout, disconnect, interrupt, stale turn, duplicate response, and late UI action close local UI
  authority and remain auditable without claiming that the remote side consumed a response;
- on deadline or UI-queue loss before an operator decision, if the same child generation and
  transport are still valid, atomically claim one method-specific decline write. If that write is
  impossible or its consumption is uncertain, hand ownership to Active Turn Exit And Restart
  Recovery for interrupt or child termination and preserve `resolution_unknown`; never close the UI
  while leaving a live upstream request with no settlement owner;
- redact raw command/permission values from general logs while retaining bounded operator-visible
  facts and decision provenance;
- update the protocol classification contract so new request methods fail a test until explicitly
  supported or rejected.

**User outcome**

The TUI presents command and permission choices actually offered by official Codex instead of
collapsing them to a generic approve button. One-shot, session, persistent, decline, and cancel
semantics remain visibly distinct; local write ownership is race safe and remote consumption is not
overstated.

**Required proof**

- schema fixtures for every command decision and bounded permission profile, plus non-empty,
  missing, null, empty, unknown, mixed-unknown, and malformed `availableDecisions` cases;
- round-trip response serialization against pinned official app-server fixtures;
- command and permission modal tests at narrow/wide terminal sizes;
- second-confirmation and scope-copy tests for persistent decisions;
- duplicate, timeout, disconnect-before-write, disconnect-after-write-before-ack, interrupt,
  stale-child/turn/workspace, callback-ID, late-action, and remote-observation race tests;
- timeout-before-decision with a live child proves either one decline write or an explicit recovery-
  owned interrupt/termination transition, never an ownerless pending request;
- UI-queue loss and missing/mismatched required thread/turn/item identity prove the same decline-or-
  recovery settlement without presenting an approval;
- unattended decline tests across planning/parallel and disconnected main runtime, plus unchanged
  file-change/user-input/MCP decline tests;
- new command/permission source-to-sink rows in the release-invariant canary matrix;
- one real disposable app-server capture for command or permission, decline, cancel, and session-
  accept behavior; unsupported live cases stay labeled unverified.

### Existing P0 Dependency: Active Turn Exit And Restart Recovery

Do not create an `attach` subsystem beside app-server. Amend the existing
[contract](../jcode/gap-matrix.md#p0-active-turn-exit-and-restart-recovery):

- distinguish clean TUI detach, controlled exit, app-server child loss, stdio transport loss, Akra
  process restart, and official thread becoming inactive;
- define a field-authority map before recovery: official current thread/status/turn identity,
  returned stored `ThreadItem` history, event-only delta/tool detail, and Akra durable state are
  distinct sources with explicit omission rules;
- after a gap, query every available official projection required by that map before accepting more
  input, but call a field authoritative only where the pinned protocol guarantees it;
- compare expected thread/turn identity and durable pending-submit identity before resuming;
- mark event-only detail omitted by `thread/read` as `history_incomplete`; mark missing or
  contradictory active-turn identity `unknown` or `recovery_required`;
- never synthesize missing deltas, replay a prompt automatically, or infer completion from quiet;
- prove that the next submission creates exactly one turn after unique reconciliation.

Required proof remains in the existing slice, with added event-gap, external-loss, stale projection,
field-authority, lossy-thread-read, `history_incomplete`, terminal-event-loss, and bounded-saturation
fixtures.

### Existing P0 Dependency: Protocol-Native Live Execution Rail

Amend the existing [live rail](../jcode/gap-matrix.md#p0-protocol-native-live-execution-rail):

- keep a bounded hot window for current items and recent turns;
- label truncation and expose official older-turn loading rather than pretending the window is full
  history;
- retain full aggregated turn diff in bounded drilldown storage under the existing policy;
- project child/fork identity and approval/elicitation phase without parsing text;
- coalesce progress deltas and preserve terminal/error transitions under saturation;
- test dense diff, 10k-message catalog metadata, resize, and burst streaming without adding another
  transcript store.

### Existing P0 Dependency: Native Performance Evidence Contract

Keep one [performance artifact schema](../jcode/gap-matrix.md#p0-native-performance-evidence-contract).
Add OpenCode-inspired profiles for dense diff, long-session hydrate/load-more, event burst, palette
filtering, and repeated resize. Measure Akra plus all app-server descendants. Do not use OpenCode's
bundle size or manual Chromium results as Akra gates, and do not publish a winner until both products
run through a compatible authenticated harness.

### Existing P2 Sub-Slice: Protocol-Native Forked Session Lineage

**Prerequisites**

- stable official session selection/resume path;
- [Active Turn Exit And Restart Recovery](../jcode/gap-matrix.md#p0-active-turn-exit-and-restart-recovery)
  complete so ambiguous outcomes and active-turn ownership have one recovery model;
- app-server protocol drift checks green;
- no dependency on the planned semantic search or remote-node work.

**Owned boundary**

- the existing [Official Session Search And Provenance](../jcode/gap-matrix.md#p2-official-session-search-and-provenance)
  owns catalog, lineage projection, Admin detail, and stable navigation;
- this bounded sub-slice owns only the app-server fork request/response adapter path;
- conversation/session application service;
- TUI current-session and Sessions-overlay fork action.

**Contract**

- call official `thread/fork` with source thread and optional last turn; do not copy messages in Akra;
- preserve returned thread ID and `forkedFromId`, plus source/fork display title and creation time;
- verify the returned lineage matches the requested source before selecting the fork;
- make source and fork independently resumable through the existing official catalog;
- leave the source thread unchanged and prove no Akra planning or delivery authority is transferred
  implicitly;
- when a source session is linked to an accepted task, create an explicit provenance edge for the
  fork; do not silently reassign the lane, lease, branch, PR, or delivery record;
- prevent fork during an unresolved approval/elicitation or active submission unless app-server
  explicitly supports the requested turn boundary and Akra can prove it;
- use idempotency/CAS around the local action so timeout retry cannot create unbounded duplicate
  forks; ambiguous server outcomes require catalog reconciliation and operator choice;
- display broken, missing, imported, or non-resumable lineage without inventing a parent;
- do not add deprecated rollback semantics in this slice.

**User outcome**

The operator can branch an official Codex conversation at a chosen turn, see exactly where it came
from, return to either thread, and keep delivery ownership explicit.

**Required proof**

- adapter fixtures for full fork, last-turn fork, malformed lineage, timeout, duplicate retry, and
  protocol drift;
- source-unchanged and independent-resume integration tests;
- planning/task/lease provenance tests that prevent implicit authority transfer;
- restart/catalog reconciliation and ambiguous-outcome tests;
- TUI narrow/wide fork-action snapshots; lineage navigation/Admin projection remain acceptance of
  the parent P2 session-provenance slice;
- one real app-server fork/resume capture with source and child thread IDs redacted only where
  required by the evidence policy.

### P2: Contextual Command Registry Metadata

This extends the shipped `:` palette; it does not create a parallel application command bus.

**Owned boundary**

- `src/adapter/inbound/tui/app/inline_shell_commands.rs` registry metadata;
- typed TUI context-to-availability reducer;
- palette, help, and optional leader/which-key presentation;
- existing shell controller/executor and overlay key routing.

**Contract**

- keep one registry entry as the source for typed command, alias, palette, help, and discovery
  binding;
- add stable ID, group, argument synopsis, availability predicate, bounded unavailable reason, and
  optional discovery binding;
- predicates consume typed current state and have no I/O or mutation;
- accepting an item revalidates availability against the latest state before execution;
- disabled items remain searchable and explain why; hidden is reserved for capabilities unavailable
  in the current build/backend, not transient state;
- leader/which-key is a discovery surface only and invokes the same registry action;
- conflicts, unreachable bindings, duplicate aliases/IDs, and palette-only executable actions fail
  tests;
- approvals, editor-local keys, destructive confirmations, and overlay-owned navigation do not move
  into the global registry;
- preserve `:help` and direct typed commands in terminals that cannot or should not use leader keys;
- cap rendered rows and collapse groups deterministically at narrow widths without changing the
  input buffer or terminal geometry.

**User outcome**

The operator can find the right action from current context, understand why an action is unavailable,
and learn shortcuts without memorizing a second command vocabulary.

**Required proof**

- registry uniqueness, alias, conflict, reachability, and state-matrix tests;
- stale availability revalidation and approval-modal non-bypass tests;
- typed command, palette, help, and discovery binding equivalence tests;
- prompt-editing/overlay ownership regression tests;
- narrow/wide terminal snapshots and real resize capture;
- palette filtering/input-latency samples added to the existing performance contract.

### P2: Inspectable File-Change Approval

**Prerequisites**

- [Protocol-Native Live Execution Rail](../jcode/gap-matrix.md#p0-protocol-native-live-execution-rail)
  retains the complete bounded patch and target identity required for inspection;
- Command And Permission Choice Fidelity supplies the child/request ownership lifecycle.

**Owned boundary**

- app-server file-change approval parser/serializer;
- bounded patch inspection model;
- TUI file-change modal and local response ownership.

**Contract**

- preserve the current automatic decline unless the entire patch, target identity, scope, and
  advertised decisions fit the bounded inspection contract;
- never approve a truncated patch, unknown target, stale child/turn, or mismatched item;
- use the same local write lifecycle and `resolution_unknown` behavior as command approval;
- keep unattended planning/parallel paths on automatic decline;
- add file-content canary rows in this feature PR, allowing the explicit inspection modal and
  forbidding unrelated persistence/projection sinks.

**Required proof**

- complete/truncated/oversized/binary/path-drift/stale-turn fixtures;
- narrow/wide patch modal snapshots and explicit decline fallback;
- disconnect-before/after-write races and one real app-server capture when offered.

### P2: Typed User Input And MCP Elicitation Forms

**Prerequisites**

- the approval ownership lifecycle is shipped;
- the release-invariant source/sink matrix can classify form content separately from credentials.

**Owned boundary**

- typed user-input and MCP elicitation request/response domain;
- app-server server-request parser/serializer;
- TUI form and URL-decision modal.

**Contract**

- keep user-input and MCP elicitation distinct because their identities and response shapes differ;
- support only bounded string, enum, number/integer, boolean, required/default/range, and format
  metadata; unsupported schema features decline instead of becoming a free-form prompt;
- treat URL elicitation as an explicit external-open decision with visible host and scheme
  allowlist; never fetch or open automatically;
- use child/method/thread/turn/workspace/elicitation ownership and local write-state semantics from
  the approval slice;
- parallel, planning, CLI, Telegram, automation, and disconnected runtimes continue to decline;
- add form-content and any new credential-handle canary rows in this feature PR.

**Required proof**

- fixtures for every supported primitive, required/default/range behavior, unknown schema, URL
  scheme/host, stale identity, timeout, and disconnect-after-write;
- narrow/wide form snapshots and unattended decline tests;
- live capture only for cases actually offered by a disposable app-server/MCP setup; all other cases
  remain fixture-verified or explicitly unverified.

### Existing P1 Automation Amendment: Stable Outcome Deep Links

Do not add another automation slice. Amend the existing Canvas sequence:

```text
trigger definition -> intake event -> run/attempt -> accepted direction/task
-> lease -> official session/fork -> source branch/SHA -> validation -> PR/review
-> delivery attempt -> integration -> cleanup
```

Schedule and webhook UX may deep-link into this chain, but no prompt string, GitHub event, or agent
commit gains delivery authority. The existing idempotency, signed webhook, reconciliation, and
authoritative-delivery prerequisites remain unchanged:

- `Automation Intake Core -> Schedule Intake / Signed Webhook Intake`;
- `Frozen-Source Validation -> Critical Review Response -> Authoritative Delivery Outcomes
  -> Automation Run Reconciliation`;
- `Guarded Admin Control Actions -> Admin Automation Run Trace`.

### Existing P2 Session Amendment: Lineage And Load More

Amend [Official Session Search And Provenance](../jcode/gap-matrix.md#p2-official-session-search-and-provenance)
to include official fork/child lineage, provider-cursor load more, stable source/fork navigation, and
trigger/task/PR deep links. Keep imported/non-resumable sessions read-only and do not add semantic
embeddings in this slice.

## Comparative Experiments

### Permission And Secret Canary

Reuse the existing Canvas [experiment](../agent-canvas/gap-matrix.md#permission-and-secret-canary)
as the current-surface baseline. Each owning P1/P2 or automation/remote feature adds only its new
matrix rows in that feature PR. Add:

- direct read versus shell read of the same disposable `.env` canary under official Codex policy;
- encoded secret variants in Admin, logs, SQLite, Git, and PR adapter captures;
- authenticated and unauthenticated Admin route matrices;
- one-shot, session, persistent-policy, decline, cancel, and permission cases with the P1 slice;
- file-change rows with Inspectable File-Change Approval and MCP/user-input/URL rows with the typed-
  forms P2 slice;
- explicit `not offered`, `unsupported`, and `unverified` results instead of substituting fixtures
  for a live path.

### Disconnect And Reconciliation Matrix

For main and parallel sessions, inject loss before submit write, after write/before response, during
streaming, during approval, after terminal event/before persistence, under bounded receiver
saturation, and during shutdown. Assert each available official projection and its declared
omissions, Akra durable state, displayed `history_incomplete`/recovery status, allowed next action,
and duplicate-turn count for each case.

### Long-Session Interaction Profile

Create deterministic fixtures with 10k messages, 1k tool items, dense diff, child/fork lineage, and
bursty output. Measure hydration, palette search, selected timeline, load more, resize, memory, and
event coalescing through the existing performance harness. The fixture proves bounds and regression;
it does not simulate model quality.

## Recommended Order

1. Land the existing Native Performance Evidence Contract when its active validation lane is free;
   add the OpenCode-inspired profiles without changing the artifact schema.
2. Establish the known-canary matrix for currently shipped Akra-owned sinks as quality evidence;
   future surfaces add rows in their own PRs and do not block on nonexistent matrix entries.
3. Complete Active Turn Steering, then Active Turn Exit And Restart Recovery with field-authority
   gap reconciliation and explicit history omissions.
4. Complete the Protocol-Native Live Execution Rail, then Parallel Current Activity Envelope and
   Truthful Diorama State Machine in their existing dependency order.
5. After recovery is stable, implement Command And Permission Choice Fidelity in a separate protocol/
   TUI lane; file-change and typed forms remain fail closed until their P2 prerequisites land.
6. Complete Frozen-Source Validation Evidence, then Critical Review Response Loop, then
   Authoritative Delivery Outcomes. Run reconciliation cannot claim terminal delivery before this
   chain is green.
7. Automation Intake Core may then branch into Schedule Intake and Signed Webhook Intake. Automation
   Run Reconciliation follows Authoritative Delivery Outcomes; Admin Automation Trace follows
   Guarded Admin Control Actions. Add stable deep links without changing that DAG.
8. Add Admin operational metrics/controls and the existing read-only remote-node sequence only in
   their documented order.
9. After Active Turn Recovery, extend Official Session Search And Provenance with the bounded
   protocol-native fork sub-slice and lineage/load-more projection.
10. Treat Inspectable File-Change Approval, Typed User Input/MCP Forms, and Contextual Command
    Registry Metadata as P2 follow-ups after the P0 protocol/delivery wedge is measured and green.

The quality-evidence, performance, and protocol/live-event lanes can proceed in parallel only when
their worktrees and validation artifact ownership do not overlap.

## Success Audit

This comparison has produced value only when later Akra evidence proves all of the following:

- every required current or feature-added credential/content canary reaches its declared positive
  sink, no canary appears in its forbidden surfaces, and the matrix cannot pass from missing
  captures;
- every supported official approval or elicitation choice remains typed, scoped, inspectable,
  redacted, and locally written at most once, while unsupported input fails closed and uncertain
  remote consumption remains `resolution_unknown`;
- event/child loss reconciles protocol-guaranteed fields without duplicate prompts or invented
  deltas, while omitted event-only detail remains `history_incomplete`;
- a fork uses official `thread/fork`, preserves `forkedFromId`, resumes independently after restart,
  and never transfers task/delivery authority implicitly;
- palette, typed command, help, and discovery binding share one registry and revalidate current
  availability;
- live diff/timeline/session projections are bounded, visibly truncated, and can load official older
  history;
- performance evidence covers the complete Akra/app-server process tree with raw samples and no
  unsupported cross-product claim;
- automation deep links lead to authoritative source, review, integration, and cleanup outcomes;
- the Admin game view moves only for persisted semantic transitions;
- no provider runtime, arbitrary plugin host, browser IDE, optional-auth control server, shared-
  checkout background authority, or prompt-only delivery displaced these goals.
