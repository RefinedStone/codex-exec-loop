# Upstream Codex To Akra Gap Matrix

This document turns the [OpenAI Codex v0.144.1 audit](analysis.md) into Akra decisions. Immutable
source, released-binary probes, raw samples, Akra baseline links, and limitations are in
[evidence.md](evidence.md). `Ahead` means stronger evidence for the named boundary at the pinned
snapshots, not a total product or security score.

Upstream Codex is both a dependency and a reference client. The matrix therefore distinguishes:

- **project**: preserve official runtime meaning through an Akra application boundary;
- **adopt**: use an upstream pattern without duplicating authority;
- **reject**: do not add an Akra subsystem for upstream-owned behavior;
- **differentiate**: invest in accepted intent, isolation, validation, review, integration, and
  operator truth;
- **observe**: track an upstream risk without pretending Akra can repair it internally.

## Immediate Finding

P0-A terminal truth is present on `prerelease` at `5a9d342f`: current live paths preserve typed
completed, failed, interrupted, and unknown outcomes instead of collapsing matching
`turn/completed` notifications to generic success. P0-B applied-envelope projection is present at
`7960ecca`, and P0-C1 closed item identity is present at `7514eb05`. This atomic slice implements
P0-C2 bounded progressive activity. P0-D1's bounded progressive activity rail is present at
`b53559ca`, P0-D2's transient retained Diff/Output inspector is present at `594859a6`, and P0-D3's
typed priority rail is present at `89264f6e`. P0-D4 now has both its deterministic supplemental E3
capture and a reviewed
[E1-E4 physical-resize artifact](../../validation/artifacts/pr-1926-physical-resize/README.md)
bound to source-build candidate `0a5f06ea`. The E1-E4 set satisfies the shared terminal-primitive
manual reviewer obligation. Every capture remains `supplemental-unmatched`, does not count toward
`terminal-baseline`, and is not released Akra runtime activity evidence. P0-D remains partial only
because that released Akra runtime activity capture is absent.
Durable recovery, validation/delivery projection, and broader surface rendering remain separate
owners rather than implied consequences of P0-A through P0-D3.

## Authority Boundary

| Authority | Official Codex owns | Akra owns | Forbidden shortcut |
| --- | --- | --- | --- |
| Model execution | provider selection, request protocol, reasoning, service tier, reroute | requested/effective projection and operator selection from official capabilities | Akra provider runtime or model registry |
| Tools | shell, command, patch, filesystem, web, image, dynamic tools, MCP, apps/plugins/skills/hooks | bounded typed activity and approval projection | Akra tool engine or raw RPC proxy |
| Safety | runtime sandbox/approval semantics and account/config storage | safer child/shell environment launch, authority routing, redaction, delivery policy | claiming Akra controls upstream internal sinks |
| Sessions | rollout persistence, resume, rollback, compaction, thread/fork semantics | catalog/search/provenance projection and planning/delivery links | duplicate transcript/session database as authority |
| Runtime agents | Codex subagent/collaboration execution and inheritance | accepted tasks, worktree leases, delivery state, resource policy around lanes | treating subagent completion as task integration |
| Review | Codex review mode and review turns | GitHub review evidence, critical-response loop, frozen-head approval | treating `EnteredReviewMode` as reviewed delivery |
| Remote | app-server/remote-control transport and runtime API | bounded authenticated Admin/CLI/Telegram/automation DTOs | internet-facing app-server method forwarding |
| Delivery | Codex may invoke Git/GitHub tools | source freeze, validation, PR/check/review/rebase/integration/push/cleanup | worker prose or generic turn success as delivery proof |

## Relative Matrix

| Dimension | Upstream Codex v0.144.1 | Akra `226e4794` | Verdict | Ownership | Consequence |
| --- | --- | --- | --- | --- | --- |
| Runtime topology | default TUI uses typed in-process app-server; optional UDS/remote | child app-server plus external JSON-RPC | upstream structurally ahead; measured winner unknown | existing performance/recovery P0 | measure full trees and test UDS viability without promising daemon continuity |
| Protocol vocabulary | 87/68/10/1 stable methods; capability-gated experimental expansion | normalized 0.144.0 experimental definitions match 0.144.1, runtime disables experimental | vocabulary current, contract blurred | release invariant | pin stable and experimental artifacts plus negotiated capability separately |
| Terminal truth | completed/interrupted/failed/inProgress plus typed error and retry flag | generic completed; every error terminates | upstream decisively ahead | existing Live Rail P0 | fix before any completion-derived automation |
| Live item semantics | 18 item kinds plus started/delta/diff/plan/usage/reroute events | most payloads drop or become generic warning | upstream ahead | existing Live Rail P0 | closed typed projection with unknown, bounds, and redaction |
| Applied runtime state | thread response and reroute carry effective envelope | start/resume reads only thread; requested state can look effective | upstream ahead | Live Rail P0 then new model P1 | expose requested versus applied versus rerouted |
| Model catalog | official models, defaults, efforts, input modalities, tiers | hard-coded models and closed effort enum | upstream ahead | new P1 | consume stable `model/list`; never own provider routing |
| Steering | official `turn/steer` with expected turn identity | running input waits | upstream ahead | existing steering P0 | same-connection CAS-safe steering |
| Recovery | official persistence/reconstruction; TUI embedded/remote/daemon shapes | reconnect spawns a child; active outcome reconciliation incomplete | upstream ahead | existing recovery P0 | durable intents, typed terminal reconciliation, bounded UDS experiment |
| Session catalog | cursor/filter/lineage/fork/resume and rich provenance | partial list/read/resume; cursor and lineage lost | upstream ahead | existing session P2 | stable list/filter/lineage first, fork second, experimental paging excluded |
| Compaction | mature official authority with a large-input edge risk | no duplicate compactor | correct boundary | observe through recovery | project compaction/error/context facts; do not rebuild |
| Runtime subagents | stable V1 bounded agents; V2 under development | worktree-isolated parallel lanes | complementary | Live Rail then parallel activity P1 | project official activity; keep lease/delivery authority distinct |
| Approval UX | rich context and choices, but defaults/queue have edge limitations | reduced approval semantics and no remote-clear state | upstream ahead | existing approval P1 | preserve offered choices, FIFO ownership, `remote_cleared`, authoritative outcome |
| Shell environment | upstream default inherits all and disables default secret excludes | child scrubbed and model shell `core` by default with explicit elevated override | Akra structurally ahead | existing canary invariant | prove launch overrides and effective envelope on every runtime path |
| Filesystem sandbox default | no trust decision is read-only; either recorded decision selects workspace profile | normal main/parallel work turns default workspace-write; setup/planning read-only | upstream safer for normal work before a decision exists | existing permission-choice P1 | record decision/profile choice and show requested/applied policy |
| Host API authority | stable fs/shell methods can act with local-user authority | bounded application surfaces; raw app-server not public | Akra boundary is safer by design | invariant | never proxy raw JSON-RPC or upstream structs |
| Linux install compatibility | simple official tar lacks bundled `bwrap`; fails without system helper | Codex is an external prerequisite; no exact install-shape receipt identified | upstream artifact defect can break sandboxed runtime use | compatibility release invariant | narrow operator prerequisite now; prove supported install shapes later; do not vendor helper |
| Auth storage | plaintext default; new Unix files request 0600 but existing mode is not repaired; alternatives exist | delegates Codex authentication | upstream-owned operator risk | canary observation and operator prerequisite | do not copy credentials; observe storage choice/residue/mode without payload |
| Remote transport | guarded WS/UDS, but WS/daemon/remote-control remain experimental | Admin capability/session auth, Telegram, GitHub identity checks | different | guarded Admin P1 | keep application broker and reject raw attach as product default |
| TUI runtime breadth | reference surface for most Codex features | narrow live projection, strong host scrollback and delivery overlays | upstream ahead on interaction | Live Rail plus TUI prioritization | retain inline scrollback; add high-priority typed rail/drilldown |
| Admin operations | no equivalent self-hosted delivery Admin found | planning/review/metrics/diorama with incomplete live runtime truth | Akra ahead in owned workflow | existing Admin P1s | one application state across TUI/Admin/CLI/Telegram |
| Reviewed delivery | no matching worktree/PR/review/rebase/cleanup invariant | verified default path plus explicit high-risk bypasses | Akra ahead | delivery P0/P1 | expose evidence and bypass provenance as first-class state |
| Performance proof | direct lower bound and in-process design; no tracked interactive SLO | existing benchmark contract, no current comparable artifact | winner unknown | performance P0 | one versioned PTY/protocol/process-tree/backlog artifact |
| Release assurance | strong signing/package workflow; exact release gate does not depend on normal tests, simple tar helper gap | locked builds, deterministic/checksummed Akra bundle; Codex is external prerequisite | mixed | release/startup invariants | test resolved upstream runtime and helper, not only Akra archive integrity |

## Adopt

### Typed Semantic Boundary

Use official app-server vocabulary at the outbound edge, then reduce it into small application types
that preserve operational meaning. The projection must be closed: every known wire variant maps to
a typed fact or an explicit ignored/redacted decision, and every unknown variant becomes visible
protocol drift. Method-name classification alone is insufficient.

### Requested, Applied, And Observed State

Every execution view should distinguish:

```text
requested by Akra/operator
-> accepted response from app-server
-> effective thread/turn envelope
-> later reroute/settings/status observation
```

The effective state includes model/provider, reasoning effort, service tier, cwd, sandbox, approval
policy/reviewer, permission profile when negotiated, and relevant environment classification. On
the stable path, a custom config default may apply while named profile provenance remains unavailable;
the legacy sandbox projection must not be relabeled as that custom profile. A requested field is not
proof that upstream applied it.

### Bounded Client Semantics

Adopt bounded queues, lag/overload markers, shutdown deadlines, correlation IDs, and negative
capability tests. Akra-owned adapters and application services must not add an unbounded queue after
upstream supplied backpressure. Delta-heavy streams need coalescing or bounded loss policy that
never discards terminal, approval, item-boundary, or delivery-relevant facts.

### Official Session Provenance

Use stable official list/read/resume/fork semantics and preserve lineage, source, cwd, Git, agent,
history, archive, ephemeral, and cursor facts. Link those facts to Akra planning and delivery IDs;
do not turn the link into a second session authority.

### Runtime Compatibility Evidence

Treat the resolved Codex binary, app-server version, schema/handshake, sandbox helper, auth state,
and effective launch policy as an install compatibility receipt. The official simple-tar
Bubblewrap failure proves version output alone is not readiness evidence. Akra's archive does not
contain Codex, so the response is a tested prerequisite and diagnostic, not vendoring `bwrap` into
the Akra bundle.

## Reject

### Runtime Duplication

Do not add provider clients, model execution, tool registries, shell/patch/fs engines, MCP hosting,
compaction, rollout persistence, or semantic memory. Upstream owns them and changes quickly.

### Raw App-Server Exposure

Do not forward generic JSON-RPC, filesystem, shell, command, process, plugin, config, or credential
methods through Admin, Telegram, webhooks, or a remote node API. Authentication does not narrow an
authenticated app-server client's local-user authority. Public DTOs stay allowlisted and map to
application services.

### Experimental Continuity Claims

Do not market app-server daemon, WebSocket, remote-control, realtime, remote environments, or
experimental history paging as a supported Akra continuity contract. A bounded UDS experiment may
inform the existing recovery design, but disconnect, updater restart, configuration replay, and
platform support must be reproduced first.

### Runtime State As Delivery State

Do not equate any of these with Akra completion:

- `turn/completed` without inspecting status;
- item completion or a final assistant message;
- Codex plan completion, goal completion, review mode, or subagent completion;
- worker prose claiming tests, review, commit, PR, or merge;
- an archived official thread;
- `serverRequest/resolved` without the authoritative item/turn outcome.

### Automatic Memory Expansion

Do not add Akra embeddings or extracted personal memory while official session provenance and
accepted planning links remain incomplete. The experimental upstream memory path illustrates the
privacy, cost, rate-limit, redaction, correction, and sandbox obligations such a subsystem creates.

## Differentiate

### Runtime Truth Before Runtime Breadth

Akra should be more precise than a generic client:

- retrying, running, completed, interrupted, failed, recovery-pending, and unknown are distinct;
- item start, progress, completion, and authoritative turn outcome remain correlated;
- requested and effective execution configuration remain visible;
- unknown wire values survive as inspectable drift rather than silent success;
- raw reasoning/tool output receives explicit retention, redaction, and size policy;
- all inbound surfaces consume one application projection.

### Reviewed Operating Layer

The default Akra lifecycle remains:

```text
accepted intent
-> isolated lane and lease
-> official runtime activity
-> candidate report
-> commit and frozen source
-> local validation clear or not-required
-> publication and PR
-> trusted CI clear or not-required
-> frozen-head review/check gate
-> rebase/integration and remote verification
-> PR/source/slot cleanup
```

The explicit autonomous bypass paths retain their policy provenance and never masquerade as reviewed
delivery. Official turn success is only one input near the beginning of this lifecycle.

### Cross-Surface Fleet Operations

The TUI is the primary interactive surface, Admin is the operational fleet surface, and CLI,
Telegram, and automation are bounded commands/views. They must share terminal truth, current item,
effective envelope, approval ownership, lane/lease, validation, review, integration, and recovery
state. The game diorama may animate only persisted semantic transitions.

## Release Invariant Amendments

### Golden Protocol Fidelity

Promote the existing Canvas
[Golden Protocol Fidelity Trace](../agent-canvas/gap-matrix.md#golden-protocol-fidelity-trace)
from a comparative experiment into an Akra release invariant for the shipped app-server adapter.

**Contract delta**

- pin exact supported Codex CLI/app-server versions and record the resolved binary digest;
- generate and retain normalized stable and experimental schema artifacts separately;
- initialize the normal Akra path with `experimentalApi: false` and fail a test if an application
  contract depends on an experimental method or field without an explicit opt-in product decision;
- inventory stable/experimental request and notification counts as orientation only;
- maintain a versioned manifest whose rows declare capability stage, owner slice, source fixture,
  active sinks, and `planned` or `required` gate state;
- gate only rows for currently shipped paths. A future surface remains `planned` and cannot count
  green; its owning feature PR supplies captures and promotes the row to `required`;
- P0-A promotes mutually exclusive live paths for success, interrupted, failed,
  retrying-then-success, nonretrying-error-with/without-terminal, drifted terminal, and terminal
  sink failure;
- P0-B promotes requested/applied/rerouted envelope rows; P0-C1/C2 promote item and delta rows;
  recovery, approval, parallel, delivery, Admin, and later inbound surfaces promote only their own
  source-to-sink rows when implemented;
- fail on a newly generated method/item/status, missing expected capture, duplicate terminal,
  correlation mismatch, or any path that promotes unknown to success;
- keep stable-runtime and opt-in experimental fixtures separate even when their normalized
  definition bodies match.

**Required proof**

- deterministic fake app-server fixtures and source-to-sink manifest;
- one released-binary stable handshake plus negative experimental-capability request;
- exact schema content hashes and version metadata;
- test that the current wrong `error.params.message` and generic terminal fixtures fail before the
  implementation fix and pass only with nested error/status preservation;
- a release gate that cannot pass from an empty/stale required fixture directory and cannot treat a
  planned row as passing evidence.

### Known-Canary Bounded Egress Matrix

Amend the existing OpenCode
[release invariant](../opencode/gap-matrix.md#known-canary-bounded-egress-matrix), not its authority.

**Contract delta**

- treat upstream's default `shell_environment_policy.inherit=all` and
  `ignore_default_excludes=true` as an explicit negative control;
- prove every currently shipped main, resumed, planning, and parallel child launch applies Akra's
  default process scrub and `shell_environment_policy.inherit=core` plus secret excludes;
- declare recovery, review-response, automation, and later credential-bearing paths as planned rows;
  each owning feature PR adds captures and promotes only its rows to required;
- record the exact elevated `process-env=all` and shell-inherit-all overrides separately and never
  inherit the default green result;
- add canaries for API/OAuth/PAT/private-key shaped names, proxy URLs, cloud credentials, SSH agent
  handles, and arbitrary non-pattern secrets while keeping the claim bounded to named values;
- verify the applied thread envelope and a disposable command environment, not only constructed
  process arguments;
- add a disposable `$CODEX_HOME/auth.json` synthetic canary and separate file, keyring, and
  ephemeral upstream storage observations; cover `Auto` keyring success, file fallback, residual
  `auth.json`, and effective mode without reading or persisting credential payload; environment
  scrub does not protect a readable file and Codex does not repair a loose pre-existing mode;
- exercise direct-read and shell-read paths and classify the official transcript/tool sink as
  supported, unsafe observation, or unknown without claiming Akra can prevent an authorized read;
- prove login-profile processing cannot reintroduce named parent canaries into the default tool
  child, and record timeout plus process-tree cleanup;
- add an upstream-observation row for rollout parent/file modes under synthetic `0022` and `0077`
  umasks, plus mode retention after any official archive/compression path exercised by the fixture;
  this proves observation only and cannot turn Akra into the rollout authority;
- observe but do not claim control over official `auth.json`, rollout, MCP/plugin, or provider
  internal storage, and keep the private `$CODEX_HOME` prerequisite in the operator runbook;
- retain raw app-server filesystem/shell/process methods as forbidden public-DTO sinks.

**Required proof**

- argument construction and released-binary command-environment captures;
- bounded auth storage-choice/residue/mode and rollout parent/file/archive-mode observations that
  never capture secret or transcript contents;
- exact positive-capture and forbidden-sink rows for every required current runtime kind;
- TUI/Admin/CLI/Telegram/log/SQLite/Git/GitHub scans from the existing invariant;
- negative test showing removal of the Akra shell-policy override exposes the upstream default and
  fails the matrix.

The current-surface manifest, scanner, positive captures, and forbidden-sink gate are row 9 in the
delivery order. They are not assumed green from this documentation audit.

### Codex Install Compatibility Receipt

Narrow the operator prerequisite immediately to a Linux Codex install with either the official
complete-package helper or a compatible system `bwrap`. Amend the automated native packaging smoke
contract after the existing Frozen-Source Validation Evidence slice defines its evidence schema.
Until then, the local negative probe and manual runbook no-op are audit/operator evidence, not a
green release receipt. Akra packages itself and declares Codex plus completed login as external
prerequisites; do not turn this finding into an Akra package manager or add upstream `bwrap` to Akra
archives.

**Contract delta**

- identify the exact resolved `codex` executable, version, digest, install-shape classification,
  sandbox helper provenance, and probe exit without logging credentials or arbitrary environment;
- run a bounded read-only/no-write sandbox no-op against every claimed supported install shape;
- distinguish system helper, adjacent complete-package helper, missing helper, helper/version
  mismatch, sandbox denial, unsupported platform, and probe timeout;
- cover the official simple Linux tar without system `bwrap`, complete package with bundled helper,
  and system-helper layouts in the release matrix where available;
- document the exact Linux Codex install/helper prerequisite in the operator runbook;
- keep compatibility evidence separate from Akra archive checksum/member verification and from
  source-test proof;
- add a `StartupDiagnostics` sandbox-readiness state only if Akra claims runtime support for an
  arbitrary operator-supplied install shape; `akra doctor` may display the bounded receipt, while
  normal TUI/Admin rendering must not silently run reachability or doctor work.

**Required proof**

- disposable released-binary Linux capture reproducing the simple-tar failure;
- complete-package and/or system-helper positive capture before marking that Linux row green;
- a receipt bound to Akra source SHA, Codex digest/version, platform, install shape, helper
  provenance, command identity, time, exit, and content hash;
- missing/stale/wrong-SHA/negative receipt fails the claimed compatibility row;
- optional startup/doctor projection tests if that later product path is added;
- no secret, raw environment, or unbounded doctor payload in the receipt or public DTOs.

## Existing P0 Amendments

### Native Performance Evidence Contract

Amend the existing
[contract](../jcode/gap-matrix.md#p0-native-performance-evidence-contract):

- include the direct released app-server lower-bound sequence `spawn -> initialize -> account/read
  -> thread/list` as a separate noninteractive stratum;
- record default plugin startup/background network processes in that stratum, and add a separately
  labeled `features.plugins=false` control rather than mixing it with default-path samples;
- benchmark current Akra child/JSON path and the upstream reference TUI's embedded typed path on the
  same machine without converting the result into a supported Akra topology;
- record time to first meaningful frame, ready-to-submit, echo, first protocol item, first text
  delta, terminal projection, and recovery notice;
- count the full Akra/app-server/worker process tree and separate RSS/PSS where the platform permits;
- add fixed delta-heavy, command-output, patch, multi-agent, slow-consumer, and queue-saturation
  workloads with backlog/lag/overload observations;
- require terminal and approval events to survive every overload policy;
- consume the later local-daemon UDS experiment as a separate comparison stratum rather than
  changing the benchmarked Akra topology inside this slice;
- preserve the ten raw lower-bound samples from this audit as orientation, not a release budget;
- make an interactive winner claim only from same-version/auth/state/geometry/run-count artifacts.

### Protocol-Native Live Execution Rail

Amend the existing
[contract](../jcode/gap-matrix.md#p0-protocol-native-live-execution-rail) in five reviewable
sub-slices.

#### P0-A: Terminal Truth

**Owned boundary**

- `src/adapter/outbound/app_server/protocol/turn_notifications.rs`;
- `src/adapter/outbound/app_server/connection.rs`, the adapter wait result, and current call sites;
- conversation runtime event types and `src/core/app/turn_stream.rs`;
- current main post-turn, planning-worker, parallel official-completion, prompt-log, and archive
  consumers only;
- deterministic protocol fixtures and source-to-sink contract tests.

**Contract**

- parse `error.error.message`, `codexErrorInfo`, `additionalDetails`, `willRetry`, `threadId`, and
  `turnId` with strict active-turn correlation;
- `willRetry: true` emits a bounded retrying fact and continues reading;
- a nonretrying error is a failed terminal candidate, but correlated `turn/completed` remains the
  authoritative turn object when transport permits;
- start a bounded terminal-grace deadline after a nonretrying error; if no final turn arrives while
  transport remains alive, return unknown/recovery-pending rather than hang or invent failure
  completion;
- parse `turn.status`, typed error, item-view completeness, timestamps, and duration;
- `completed` alone yields successful terminal state;
- `interrupted` yields interrupted, `failed` yields failed with typed bounded error;
- `inProgress` on `turn/completed`, unknown status, missing required identity, or contradictory
  error/status yields protocol inconsistency and recovery-pending/unknown, never success;
- carry a typed terminal receipt, or an equivalent closed result, from notification handler through
  connection/adapter wait to every current caller; Rust `Result::Ok` is transport success and never
  terminal completion proof;
- let that receipt retain the observed upstream outcome and a separate application-delivery
  acknowledgement; a full/disconnected sink preserves the upstream fact but marks local projection
  unconfirmed/recovery-pending and cannot reach prompt-log `completed`, continuation, or archive;
- terminal application is compare-and-set and idempotent within the live path across duplicate and
  out-of-order notifications; durable readback/child-loss CAS remains Recovery ownership;
- keep changed-file/session observations as separate facts. They may request later reconciliation
  but can neither promote nor overwrite official terminal status;
- only typed `completed` may enter current main/planning continuation, parallel
  official-completion/commit-ready, prompt-log completed, or completed archive paths.

**Required proof**

- fixtures for all statuses, retry true/false, nested/missing error, correlation mismatch,
  contradictory status/error, duplicate and out-of-order terminal, and unknown status;
- a nonretrying-error/no-final-turn fixture where live transport crosses the bounded grace deadline;
- full, blocked-until-deadline, and disconnected terminal-sink fixtures proving typed receipt does
  not collapse to success;
- handler, connection, adapter, core reducer, prompt-log, current main/planning continuation,
  parallel official-completion, and archive assertions for every live terminal path;
- one disposable authenticated released-app-server capture for normal completed and
  interrupt-to-interrupted wire shapes;
- regression that failed/interrupted/unknown cannot produce generic current-path `completed`.

#### P0-B: Applied Envelope

- extend `ThreadPrepared`/active-turn application state with requested and applied model/provider,
  effort, service tier, cwd, approval, sandbox, permission profile, source, and relevant environment
  classification;
- preserve missing and unknown values instead of substituting the request;
- consume `model/rerouted` and settings/status changes as later observations;
- keep raw provider config and credentials out of domain/public DTOs;
- prove current main, resumed, planning, and parallel paths; Recovery and later review-response
  owners add their own rows when those paths land.

**Implementation evidence in this atomic slice**

- [`conversation_runtime_envelope.rs`](../../../src/domain/conversation_runtime_envelope.rs) owns
  requested, applied, rerouted, observation provenance, and explicit projection-gap semantics;
- [`runtime_envelope.rs`](../../../src/adapter/outbound/app_server/protocol/runtime_envelope.rs) and
  its checked-in
  [`runtime_envelopes.json`](../../../src/adapter/outbound/app_server/protocol/fixtures/runtime_envelopes.json)
  fixture fail closed on malformed stable responses while preserving missing, null, defaulted,
  unavailable, bounded unknown, and closed source values;
- [`conversation_runtime_event.rs`](../../../src/application/service/conversation_runtime_event.rs)
  provides the shared application projection used by planning and parallel paths, while the Core
  reducer owns current main/resume state without reparsing app-server wire values;
- pre-thread-response settings are discarded at the response chronology boundary, later
  settings/reroute/status observations use exact thread/turn correlation, and observable loss is
  represented as a projection gap before terminal delivery can be confirmed;
- all four current paths require the stable applied cwd to equal the protected requested workspace;
  both the planning adapter and application orchestration independently block authority mutation
  when the envelope is missing, rejected, or has an unresolved projection gap, while parallel
  completion enforces the same fail-closed contract;
- adapter-local response extras use a redacted `Debug`; provider metadata, collaboration settings,
  raw instruction sources, and free-form config warning bodies do not enter the public envelope,
  prompt output, or parallel trace projection.

This slice retains envelope state for later presentation but adds no applied-envelope TUI surface;
the existing bounded thread-status copy now projects accepted typed status observations. It does
not add Admin, CLI, Telegram, persistence, restart recovery, or released-app-server capture.
Observation sequence is local to the live adapter stream rather than durable authority. Existing
opt-in prompt logging still records the protected workspace cwd and explicit skill input paths,
including the bundled planning-worker skill path; it does not record raw response
`instructionSources` or the runtime envelope.

**Required proof**

- start/resume response fixtures for requested-equals-applied, upstream override, missing/unknown
  field, and malformed envelope;
- reroute/settings/status ordering and stale-turn correlation tests;
- current main/resume/planning/parallel application snapshots with no request-as-effective fallback;
- redaction tests proving provider config and credentials never enter public DTOs or logs.

#### P0-C1: Closed Item Identity

- project item started/completed identity and bounded kinds for agent/user message, reasoning, plan,
  command, file change, MCP, dynamic tool, collaboration/subagent, web, image, review-mode,
  compaction, and unknown;
- preserve start/completion order, item/turn/thread correlation, bounded summary, and completion
  outcome without requiring every kind to receive rich UI in this PR;
- treat `EnteredReviewMode` only as Codex runtime state;
- make a newly generated item/notification fail classification until its preservation decision is
  explicit.

**Required proof**

- fixtures for all 18 snapshot item kinds, missing/mismatched identity, duplicate start/completion,
  unknown kind, and bounded summary/redaction decisions;
- memory-bound reducer tests over a large completed-item stream;
- source-to-application manifest showing preserved, bounded-redacted, explicitly ignored, and
  unknown decisions.

**Implemented in this slice**

- one adapter decoder projects all 18 stable kinds plus bounded `Unknown` for both live
  `item/started`/`item/completed` and snapshot replay;
- exact thread, turn, and item identity, provider timestamp, live started/completed or
  snapshot-observed phase, reported outcome, safe summary, and FIFO application sequence reach the
  domain/Core boundary; snapshot replay does not invent a completion boundary for active items;
- the 256-record reducer exposes truncation, invalid identity, unknown kind, duplicate boundary,
  completion-before-start, kind mismatch, and timestamp-regression state instead of upgrading any
  of them to turn or delivery success;
- immutable lifecycle snapshots are shared through `Arc` and copy on write only when lifecycle
  state mutates; loaded snapshot records hydrate the same-thread live reducer and a different
  thread preparation clears them;
- all live item identities and their first kinds use a separate 512-entry, non-evicting SHA-256
  ledger. Replay remains suppressed after lifecycle-record eviction, while kind drift retains the
  new observation and fails the stream through the longer-lived ledger; capacity exhaustion retains
  the lifecycle fact and fails the stream so missing final text cannot be
  followed by a falsely confirmed turn;
- malformed active `item/completed` payloads fail the stream before a later terminal notification
  can confirm a turn whose final text or tool fact was discarded; a first uniquely identified
  completion with a regressed timestamp still delivers its payload while retaining the anomaly;
- file changes produce changed-path and tool-activity effects only for the typed `Completed`
  outcome. Failed, declined, in-progress, unknown, and replayed file changes remain lifecycle facts;
- the source manifest classifies every current item field as preserved, bounded-redacted, or
  explicitly ignored, and compares item variants, fields, closed statuses, and notification methods
  with the checked-in generated schema;
- raw prompts, reasoning, plans, command/output/diff/path/query/image bodies, tool arguments/results,
  collaboration prompts, and review text do not enter the lifecycle projection or prompt log;
- `EnteredReviewMode` and `ExitedReviewMode` remain runtime observations only. They do not mutate
  Review Center, GitHub review, planning, validation, merge, or delivery authority.

This slice intentionally adds no progressive delta handling, rich lifecycle rail, Admin/CLI/
Telegram projection, parallel activity persistence, durable restart recovery, or released Akra
runtime capture. Those owners consume this typed contract later rather than parsing app-server
JSON again.

#### P0-C2: Bounded Progressive Activity

- add command output/terminal interaction, patch/output delta, turn diff, plan, token/context usage,
  moderation/guardian, compaction, and reroute facts where they drive operator decisions;
- coalesce high-rate deltas by correlated item while never coalescing away item boundaries,
  terminal, approval, errors, or lag/overload markers;
- define size, truncation, redaction, retention, and replay policy per progressive payload class;
- keep full diff/output drilldown separate from bounded summary state.

**Required proof**

- fixed high-rate text, command-output, patch, diff, plan, token, and multi-agent streams with
  exact code-tracked retained-payload and backlog bounds; allocator/RSS measurement belongs to the
  later native performance artifact;
- coalescing order and disconnect/overload tests proving terminal and approval survival;
- context-pressure plus diff-summary/bounded-detail fixtures; interactive full drilldown belongs to
  P0-D;
- unknown delta and oversized/non-UTF-8 payload tests.

**Implemented in this slice**

- one strict adapter manifest classifies 14 progressive notifications as handled, deprecated
  `item/fileChange/outputDelta` and `thread/compacted` as explicitly ignored, and model safety
  buffering/verification as schema-validated diagnostic-only input;
- exact thread/turn/item correlation and the P0-C1 item-kind/completion ledger gate item activity.
  A missing boundary, kind drift, or activity after completion is retained before the stream fails
  closed; stale scope is dropped without consuming an application sequence;
- unknown progressive-looking item, turn, and turn-correlated thread methods retain only a bounded
  redacted method label and payload byte count, then fail closed. Closed plan-status, patch-kind,
  and verification enums also require an explicit schema decision rather than silently becoming a
  known state;
- typed payloads cover agent draft, command tail and terminal interaction counts, patch detail, turn
  diff summary/detail, plan, token/context pressure, MCP progress, reasoning/plan counts,
  moderation size, and guardian warning. Detail bounds range from 4 KiB guardian copy to 2 MiB
  agent/diff detail, with 64 records and 8 MiB retained dynamic state overall. Cumulative token
  totals must contain the latest active-context breakdown, and only that latest total drives context
  pressure after compaction;
- opaque validated batches preserve monotonic sequence and correlation. Payload truncation and
  fully dropped observations have separate counters, UTF-8 prefix/tail bounds are exact, and all
  retained detail/identity Debug output is redacted. Batch and cross-publication projection merges
  reject counter or payload-accounting overflow before mutating retained activity;
- the application mailbox keeps an eight-event control admission budget plus ordered progressive
  segments. Core ingress repeats the policy with 16 control admissions and at most 17 progressive
  segments. Only adjacent correlated publications coalesce; every control remains an exact ordering
  boundary, and each pending layer has one 8 MiB tracked-detail bound. Older detail becomes a
  same-position history-only marker rather than crossing or removing the boundary. A newer Core
  generation prunes stale progressive backlog, while a current-generation segment admission failure
  becomes a terminal stream failure instead of silent loss;
- Core consumes each batch directly into an immutable `Arc` projection, applies history-only gaps,
  rejects stale or uncorrelated batches, and resets on each new turn/thread. Production TUI polling
  applies one Core outcome at a time instead of retaining a vector of COW snapshots; its live-agent
  buffer synchronizes from the typed projection. The removed legacy delta enum can no longer bypass
  this path;
- progressive detail is excluded from the opt-in prompt-output log and has no serialization or
  Admin/CLI/Telegram persistence path. The active-turn reducer sizes moderation and unknown payloads
  through a counting writer. Early notifications retained across the response/stream handoff may
  re-encode their existing JSON value once under the separate bounded pending-queue byte budget.

Deterministic proof includes 100,000-publication reducer/mailbox streams; fixed high-rate agent,
command, patch, diff, plan, token, and multi-agent reducers; exact
`progress -> control -> progress` ordering, eight controls interleaved with nine 2 MiB progressive
segments at application and Core ingress, approval/terminal survival, item-boundary drift failures,
all 14 handled methods, schema
shape fingerprints, exact command-line accounting across chunk boundaries, oversized UTF-8
truncation, exact source/retained/loss byte accounting including rename destinations,
cross-publication overflow rejection, post-compaction and schema-maximum context pressure, non-UTF-8
transport rejection, history-only Core application, stale-generation pruning, and secret canaries.

At the P0-C2 commit, this slice intentionally added no activity rail or Diff/Output inspector. P0-D1
later added the bounded summary rail at `b53559ca`, and P0-D2 added transient retained-detail
inspection at `594859a6`. Admin/CLI/Telegram projection, parallel persistence, durable restart
recovery, released Akra runtime activity capture, and comparative performance evidence remain
separate and absent from these slices. Deprecated compaction is represented by the P0-C1
`ContextCompaction` item
instead of a second progressive fact; model reroute remains the P0-B applied-envelope owner.

#### P0-D: Core And TUI Projection

- TUI consumes the application rail first with approval/failure, active tool/patch/plan, context,
  effective model, planning task, and the existing coarse lane summary in explicit priority order;
- narrow terminals collapse low-priority facts; full details use existing overlays/routes;
- no empty decorative rail and no raw unbounded output in a summary DTO;
- preserve host scrollback and keep transient live state out of durable terminal history;
- leave parallel persistence/Admin, diorama, delivery outcomes, metrics, CLI, and Telegram to their
  existing owner slices, which consume these types later rather than reparsing wire JSON.

**Current status (partial)**

- P0-D1 (`b53559ca32e1fddd22f13d2de4aad028feebf9fe`) adds a bounded, payload-free
  progressive summary rail with approval-first arbitration and responsive narrow/wide/vt100 proof.
- P0-D2 (`594859a621e7213356822b008a496f4d5cfca36f`) adds
  `:activity [diff|output]` / `:act`, bounded Diff and Retained Output Tail pages, truthful retention
  metadata, approval preemption, lifecycle/resize reset, terminal-safe rendering, and host-scrollback
  negatives.
- P0-D3 (`89264f6e8edf1dec31393e2fd2804054cc26662d`) completes the residual typed rail with
  `approval > terminal/recovery > active command/patch/plan > context/incomplete-history > applied
  model > active planning handoff > coarse live lane` priority, bounded control-safe dynamic facts,
  and whole-fact narrow collapse. Requested model values, stale completed-task handoffs, and
  previous-turn submitting counts are not rendered as current truth. Raw terminal error detail
  remains in the existing bounded Status transcript and is not copied into typed rail state or copy.
- P0-D4 candidate `0a5f06ea345bd5c24998527679158eaf8643d058` adds an isolated deterministic
  source-build E3 tmux detached-PTY capture covering wide/narrow/intermediate/repeat/restore and
  completion behavior. The repeated narrow frame is semantically stable after normalizing tmux
  blank-row reflow and elapsed-time text, and no transient typed fact enters host history. That
  synthetic-app-server artifact remains `approvalGrade: false` and does not satisfy an E1-E4 row
  by itself.
- Separately, the reviewed
  [E1-E4 physical-resize artifact](../../validation/artifacts/pr-1926-physical-resize/README.md)
  binds all four first-class environments to the same exact source candidate and per-platform
  binary digests. Actual Codex app-server sessions cover input, active/final state, help/redraw,
  physical shrink/restore, clear/reset, session restore, and clean exit. This satisfies the shared
  primitive-change E1-E4 manual reviewer obligation.
- These captures remain `supplemental-unmatched`, do not count toward `terminal-baseline`, and are
  not released Akra runtime activity evidence. P0-D remains partial only for that released Akra
  runtime activity capture. These slices do not claim Admin/CLI/Telegram projection, parallel or
  durable persistence, restart/reconciliation recovery behavior, or comparative latency/RSS.

**Required proof**

- core reducer tests plus narrow/wide and long-value TUI snapshots;
- vt100/ANSI frame assertions and at least one real-terminal capture;
- diff/output drilldown, resize, input, approval-priority, and host-scrollback regressions;
- no layout shift or unbounded render work under the fixed P0-C2 stream.

### Active Turn Steering

Keep the existing
[contract](../jcode/gap-matrix.md#p0-active-turn-steering) and add:

- require the terminal-truth CAS and applied envelope before enabling running-turn input;
- use the active connection/request handle with `threadId` and `expectedTurnId`;
- reject steering when state is retrying terminal-candidate, interrupted, failed,
  recovery-pending, unknown, or pending approval whose ownership would be obscured;
- preserve draft on stale/non-steerable/disconnect/overload failure;
- record requested/accepted steering separately from a new planning task or turn;
- include a real stable-capability released-app-server trace.

### Active Turn Exit And Restart Recovery

Keep the existing
[contract](../jcode/gap-matrix.md#p0-active-turn-exit-and-restart-recovery) and add:

- recovery consumes P0-A typed terminal candidates and authoritative readback instead of generic
  `Result` success;
- record last item identity, applied envelope, retry state, item-view completeness, and terminal
  provenance with the existing durable intent;
- after disconnect/child death, reconcile official thread/turn truth and apply exactly one
  completed/interrupted/failed result or retain unknown;
- never archive as completed, continue planning, retry a worker, validate, or deliver from an
  unknown or interrupted prior turn;
- apply durable compare-and-set across live receipt, authoritative readback, duplicate terminal,
  child exit, and restart;
- prove child-kill/readback/no-resubmit behavior here, not in P0-A;
- keep changed workspace/session observations separate from official turn terminal and never use
  them to upgrade an unknown outcome.

### Bounded Local-Daemon UDS Viability Experiment

Run this only after recovery correctness is green, in its own evidence worktree/PR. It is not a P0
product topology change.

- compare the current Akra child/JSON path with the official local-daemon UDS path using the same
  version, auth state, workspace, workload, and full process-tree accounting;
- cover compatible default config, non-replayable override fallback, missing/stale socket, client
  disconnect, daemon death, updater restart, app-server version drift, and process cleanup;
- record that the daemon is experimental, Unix-only, and not reboot-persistent at this snapshot;
- require terminal/recovery truth to survive every failure case;
- produce an explicit adopt/reject/defer decision. Even a positive result cannot change production
  topology without a later separately reviewed product contract.

### Frozen-Source Validation Evidence

Keep the existing
[contract](../jcode/gap-matrix.md#p0-frozen-source-validation-evidence) and add runtime and release
consumers:

- runtime turn success may authorize candidate handoff only when P0-A says `completed`; failed,
  interrupted, recovery-pending, unknown, duplicate, or semantically unclassified terminal truth
  blocks validation-ready and delivery-ready transitions;
- a successful validation command does not retroactively convert a failed official turn into
  completed worker execution;
- record the runtime terminal provenance beside exact source SHA and validation evidence;
- before a tag workflow mutates npm or GitHub Release state, require trusted native source-test
  receipts bound to that exact tag SHA, or rerun the authoritative native check from a detached
  exact-SHA checkout;
- missing, failing, stale, wrong-SHA, or wrong-policy receipts block release publication;
- keep source-test proof, Codex install compatibility proof, and archive byte/member/checksum proof
  as separate gates so one cannot stand in for another;
- preserve existing `cargo --locked` release and PR behavior with argv regression tests rather than
  opening a duplicate dependency-lock backlog.

## Existing P0/P1 Amendments

### Command And Permission Choice Fidelity

Correct and extend the existing OpenCode
[contract](../opencode/gap-matrix.md#p1-command-and-permission-choice-fidelity):

- depend on P0-A terminal truth and P0-C1 `item/started` ownership;
- before starting a normal workspace-write session without an existing authoritative trust/profile
  choice, query stable `permissionProfile/list`, present only allowed bounded summaries, and remain
  read-only unless the choice can be represented exactly;
- on the normal `experimentalApi: false` path, map only explicitly allowlisted built-in profile IDs
  with exact legacy `sandbox` equivalents and explicitly prohibit the stable generic
  `config.default_permissions` escape hatch. Named/custom typed selection and
  `activePermissionProfile` provenance require a separately reviewed experimental opt-in; discovery
  or an existing config default must never make Akra send an untyped override or fabricate applied
  profile identity;
- launching Akra alone must not be persisted as an implicit reusable trust decision;
- keep project trust/profile scope, source, time, and requested/applied sandbox visible; changing
  the profile is separate from approving one command and requires confirmation for broader scope;
- permit official network-only approval payloads that omit command/cwd while requiring their own
  network/host/protocol facts and normal thread/turn/item/request correlation;
- preserve missing/null legacy `availableDecisions` fallback separately from empty, malformed, or
  mixed-unknown arrays;
- store pending requests in arrival order; do not copy the first-party modal's possible LIFO edge;
- from any live local state, accept a matching `serverRequest/resolved` as `remote_cleared` while
  preserving the last local state, including lifecycle cleanup before a response write;
- `remote_cleared` proves only that upstream no longer has the pending request, including lifecycle
  cleanup; it does not prove accepted/declined consumption or tool effect;
- only the correlated authoritative item/turn outcome settles effect;
- cleanup-before-write, cleanup-after-write, duplicate/mismatched resolution, disconnect, timeout,
  and late UI action retain explicit provenance and never fabricate a decision result.

The state ladder becomes:

```text
pending -> locally_decided -> write_attempted -> write_confirmed
pending | locally_decided | write_attempted | write_confirmed
-> remote_cleared -> authoritative item/turn outcome
```

Any transition after `write_attempted` can remain uncertain. File approval and typed user/MCP forms
remain in their existing P2 slices.

### Admin Operational Metrics

Amend the existing
[contract](../jcode/gap-matrix.md#p1-admin-operational-metrics):

- depend on typed live/recovery truth and the existing Authoritative Delivery Outcomes slice; Admin
  metrics do not define either state machine;
- count typed completed/interrupted/failed/unknown/retrying outcomes separately;
- include app-server version/digest classification, readiness state, applied model/effort/tier,
  permission/sandbox class, protocol drift count, queue lag/overload, reconnect/recovery latency,
  and current item class;
- keep prompt/tool/reasoning bodies and raw doctor output out of metric labels and public summaries;
- invoke `codex doctor --json` only through an explicit bounded diagnostic action or scheduled
  service with timeout/cache policy, never during ordinary page rendering;
- distinguish direct runtime time, Akra overhead, model/network time, validation time, review wait,
  integration time, and cleanup time.

### Parallel Current Activity And Completion Evidence

The Canvas
[Parallel Current Activity Envelope](../agent-canvas/gap-matrix.md#p0-parallel-current-activity-envelope)
and jcode
[Parallel Activity And Completion Evidence](../jcode/gap-matrix.md#p1-parallel-activity-and-completion-evidence)
must consume P0-C1/P0-C2/P0-D types rather than adding another JSON parser.

- show official subagent/collaboration identity as runtime activity, not a worktree lease;
- preserve current item, start/progress/completion, retry, effective envelope, context pressure, and
  last-activity age;
- distinguish official turn terminal from report, commit, source freeze, validation, PR, review,
  integration, remote verification, and cleanup;
- failed/interrupted/unknown blocks success animation and distributor readiness;
- bound per-lane output and aggregate lag/overload state.

The Canvas
[Truthful Diorama State Machine](../agent-canvas/gap-matrix.md#p0-truthful-diorama-state-machine)
remains a separate PR after current activity. It consumes persisted lane/delivery semantics and may
not infer progress from TUI frames or runtime prose.

### Critical Review Response Loop

Keep the existing
[contract](../jcode/gap-matrix.md#p1-critical-review-response-loop) between Frozen-Source
Validation and Authoritative Delivery Outcomes:

- response turns consume P0-A typed terminal truth; failed/interrupted/unknown cannot close a
  critical finding or produce delivery readiness;
- every accepted response change re-freezes the candidate and invalidates stale validation,
  approval, review, and check evidence exactly as the existing contract requires;
- response-turn runtime truth and GitHub review resolution remain distinct facts;
- preserve the no-GitHub-credential response runtime and existing race/adversarial proofs.

### Authoritative Delivery Outcomes

Amend the existing
[contract](../jcode/gap-matrix.md#p1-authoritative-delivery-outcomes):

- consume P0-A terminal provenance and Frozen-Source Validation Evidence without redefining them;
- permit candidate handoff only from typed completed, then keep report, commit, freeze, validation,
  publication, PR, review, checks, integration, remote verification, and cleanup distinct;
- failed/interrupted/unknown/recovery-pending official turns cannot become ready or integrated even
  if a worker summary claims success;
- changed files, a commit, or later validation are separate facts and cannot rewrite the official
  runtime terminal outcome;
- expose explicit autonomous review/check/PR bypass policy rather than counting it as reviewed.

## New P1 Slice: Official Model Capability Projection

This is the only new product slice introduced by the upstream audit. Applied-envelope truth lands
first in P0-B; this slice replaces static selection with official capabilities.

**Owned boundary**

- new narrow outbound `ModelCatalogPort` implemented by the app-server adapter;
- application model-catalog service and cache state;
- extensible domain model/effort/capability values;
- existing TUI model/reasoning overlay and startup diagnostics;
- shared read-only Admin/CLI projection only after the TUI path is complete.

**Contract**

- call stable `model/list` after initialize and on an explicit refresh/invalidation signal;
- consume opaque cursor pages with bounded page/item limits, cancellation, and repeated-cursor
  detection; a partial catalog remains incomplete/error state rather than a silently authoritative
  picker;
- preserve model ID, display label, default, supported reasoning efforts, input modalities,
  service tiers, and bounded capability/status fields;
- represent effort as an extensible validated value, including official `max`, `ultra`, or later
  values, rather than closing the domain enum;
- select only advertised model/effort/tier combinations unless an explicit custom-ID workflow is
  separately designed;
- show requested selection and P0-B actual applied/rerouted state distinctly;
- retain the last catalog with freshness/error state on transient refresh failure; never invent a
  model or silently fall back while presenting the old choice as current;
- scope cache by app-server instance/version/account/config facts that can change the catalog;
- keep provider transport, credentials, pricing, and model execution upstream;
- remain stable-only while normal initialize disables experimental capability.

**User outcome**

The operator chooses from the models and reasoning values the running official Codex actually
advertises and can see when upstream applies or reroutes to something else.

**Required proof**

- fake `model/list` fixtures for empty, default, unknown effort, max/ultra, tier/modalities,
  pagination, repeated cursor, limit exhaustion, duplicate/malformed, refresh, and transient failure;
- application cache/invalidation tests scoped to runtime identity;
- start/resume/turn request serialization plus applied/rerouted reconciliation;
- narrow/wide TUI snapshots with long model IDs and unavailable/stale states;
- no provider credentials/raw config in domain, Admin, logs, or persistence;
- one released-app-server catalog capture against the pinned version.

## Existing P2 Amendments

### Official Session Search, Provenance, And Fork

Amend the existing
[Official Session Search And Provenance](../jcode/gap-matrix.md#p2-official-session-search-and-provenance)
and the OpenCode fork dependency:

- add cursor to list requests and consume every page with bounded cancellation/duplicate handling;
- use stable cwd/search/source/archive filters and preserve next cursor/error/freshness;
- retain session ID, forked-from, parent thread, source, agent path/nickname/role, Git info,
  ephemeral/archive/history and turn item-view completeness;
- expose lineage before implementing fork navigation;
- make fork identity and source/turn boundary explicit and link the new official thread to Akra
  planning/delivery context without inheriting delivery completion;
- project TUI catalog/detail first, then read-only Admin detail;
- do not promise experimental `thread/turns/list` or `thread/items/list` while Akra initializes
  with `experimentalApi: false`;
- distinguish unavailable detail from a complete empty history.

### File Approval And Typed Forms

Keep existing P2 ownership for inspectable file-change approval and typed user/MCP elicitation.
They depend on P0-C1 item ownership, the approval `remote_cleared` state, terminal recovery, and the
known-canary invariant. No raw upstream form or patch DTO may enter public surfaces without a
bounded application type.

## Reviewable Delivery Order

Each row is one branch/worktree/PR unless implementation discovery proves two adjacent rows are
inseparable. Later rows may begin only after their prerequisite semantic contract is merged.

| Order | Reviewable slice | Priority | Prerequisite | Exit evidence |
| ---: | --- | --- | --- | --- |
| 1 | Live terminal receipt P0-A | P0 | current schema/fixtures | typed handler-to-caller receipt, sink negatives, no current false success |
| 2 | Applied envelope P0-B | P0 | P0-A | main/resume/planning/parallel requested-vs-applied tests |
| 3 | Closed item identity P0-C1 | P0 | P0-A | 18-kind inventory, identity/order, bounded summary, drift failure |
| 4 | Bounded progressive activity P0-C2 | P0 | P0-C1 | code-tracked reducer/backlog bounds and terminal-safe coalescing |
| 5 | Core and TUI projection P0-D | P0 | P0-B and P0-C2 | reducer, narrow/wide/vt100 and real-terminal drilldown proof |
| 6 | Active turn steering | P0 | P0-A, P0-B, live identity | fake failure matrix plus real stable trace |
| 7 | Exit/restart recovery correctness | P0 | P0-A, P0-B, P0-C1 | durable CAS/readback/child-kill matrix and no duplicate submit |
| 8 | Frozen-source validation plus release consumer | P0 existing | P0-A and recovery | exact-SHA validation and pre-publication release receipt gate |
| 9 | Current-surface known-canary gate | release invariant | shipped source/sink inventory | versioned manifest/scanner and positive/forbidden current captures |
| 10 | Codex install compatibility receipt | release invariant | row 8 evidence schema | simple-tar negative plus supported positive install-shape receipt |
| 11 | Native performance artifact | P0 existing | P0-D and recovery | versioned PTY/protocol/full-tree/backlog raw samples |
| 12 | Local-daemon UDS viability | experiment | recovery and performance harness | failure matrix and explicit adopt/reject/defer evidence |
| 13 | Parallel current activity envelope | P0 existing | P0-A through P0-D | typed persisted lane activity without another wire parser |
| 14 | Truthful diorama state machine | P0 existing | row 13 | persisted semantic animation, no inferred progress |
| 15 | Official model capability projection | P1 new | P0-B | dynamic catalog picker and released capture |
| 16 | Command/permission fidelity | P1 existing | P0-C1, recovery, and row 9 | trust/profile, FIFO, offered choices, remote-clear/effect ladder |
| 17 | Critical review response loop | P1 existing | row 8 and typed recovery | re-freeze/invalidation/race proof with no stale-head delivery |
| 18 | Authoritative delivery outcomes | P1 existing | rows 8 and 17 plus typed terminal | runtime-to-delivery separation and bypass provenance |
| 19 | Admin operational metrics | P1 existing | rows 11, 13, 14, and 18 | bounded truthful metrics; no state-machine duplication |
| 20 | Session catalog/provenance/fork | P2 existing | recovery and application identity | cursor/filter/lineage/fork TUI then Admin |
| 21 | File approval/forms/additional inputs | P2 existing | row 16 and canary invariant | bounded typed UI and source-to-sink proof |

Rows 1 through 4 are implemented as separate atomic slices on `prerelease`. Row 5 remains partially
implemented by P0-D1 through P0-D4. P0-D4's reviewed supplemental E1-E4 manual artifact satisfies
the shared primitive reviewer obligation, but released Akra runtime activity capture remains
absent. The manual artifact does not count as `terminal-baseline` matrix rows. Durable
readback, child-loss CAS,
validation/delivery/Admin projection, model UI, daemon topology, and broad event rendering remain
with their later owners.

## Success Audit

Akra may claim the upstream baseline is handled only when all applicable statements are true:

- exact stable and experimental schema/capability contracts are pinned independently;
- retrying errors continue, failed/interrupted never become completed, and unknown never delivers;
- requested/applied/rerouted runtime state is visible and correlated;
- every official item/status has an explicit bounded projection decision;
- TUI, Admin, CLI, Telegram, parallel, prompt-log, validation, and delivery paths share terminal
  truth;
- app-server child and model shell environments pass the named canary matrix;
- raw app-server host APIs are absent from public remote surfaces;
- every claimed supported Codex install shape has a fresh sandbox-helper compatibility receipt;
- performance claims have same-machine interactive raw evidence and complete process-tree scope;
- approval local write, remote clear, and authoritative effect remain separate;
- official model choices come from the running stable catalog;
- session paging, lineage, and fork do not rely on disabled experimental methods;
- Codex runtime agents/review/plan/goals never replace Akra planning, lease, validation, GitHub
  review, integration, or cleanup authority;
- every implementation row lands through its own reviewed worktree/commit/push/PR/rebase-merge and
  the finished worktree is removed.
