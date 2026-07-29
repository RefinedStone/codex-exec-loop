# Runtime Architecture

[한국어](../ko/reference/architecture.md)

This is the canonical architecture and authority reference for the shipped runtime.

The arrows below are **compile-time source dependencies** (`A -> B` means A imports contracts
owned by B). They are not the order in which a command executes:

```text
adapter/inbound/tui -> core + application contracts/projections + opaque composition facade + domain
adapter/inbound/{cli,admin_api,telegram_bot} -> application -> domain
application -> outbound ports
adapter/outbound -> application ports + domain
composition -> core + application + adapter/outbound
```

The TUI line deliberately permits inward, data-only application/domain contracts and projections.
Those imports carry immutable values for mapping and rendering; they are not runtime capabilities
and do not invert the dependency direction. Production bootstrap does not receive raw application
services: composition consumes them while constructing one opaque native application object. The
adapter receives only `NativeClientRuntime` and immutable runtime-control truth. The parallel
control-plane handle, event sink, and completion mailbox remain private composition details.

The client command loop has a different, intentionally round-trip **runtime flow**:

```text
TUI intent
  -> composition/NativeClientRuntime::dispatch_client_event(NativeClientEvent)
  -> core reducer/CoreEffect or application parallel control plane/effect
  -> application use case / outbound port
  -> private bounded completion mailbox
  -> composition/NativeClientRuntime::poll_pending_client_event
  -> owning reducer
  -> snapshot/presentation event
  -> TUI projection
```

That round trip does not invert the source dependency. Core owns the effect/completion contract,
while composition interprets it with application services.

## Layer Ownership

| Layer | Owns | Must not own |
| --- | --- | --- |
| `adapter/inbound` | input mapping, rendering, local focus/editor/selection state | domain policy, durable task truth, dispatch policy |
| `core` | framework-free client runtime: command/event/effect/completion flow, process-local client state, projections, snapshots | business/domain authority, TUI/HTTP/Telegram types, application services, or concrete DB/Git/filesystem adapters |
| `application/service` | use-case orchestration, ordering gates, transactions, control-plane handles | widgets, terminal events, transport DTOs |
| `application/port` | outbound contracts required by application services | concrete integration details |
| `domain` | pure invariants, validation, decisions, state transitions | async runtime, IO, logging, UI, database, filesystem, or Git calls |
| `adapter/outbound` | app-server, DB, filesystem, Git, GitHub, and Telegram integration | business policy |
| `composition` | dependency construction and concrete wiring | domain decisions |

Mapping stays in adapters. Policy stays in domain or application services. Add a port only for a
real outbound boundary.

The Admin server owns one process-lifetime parallel control-plane handle in addition to its passive
dashboard composition. Browser control requests map only to typed enable, dispatch, refresh, and
disable commands. Effect completions return through an adapter-owned bounded channel and are
drained through the same handle, so HTTP polling never becomes a second pool or delivery policy
implementation.

## Core Runtime

`src/core` is the framework-free **Client Runtime** outside the business hexagon, at its inbound
client boundary. It is a long-lived state coordinator for the native client, not another business
layer and not a replacement for application or domain. CLI, Admin, and Telegram may call the same
application services without adopting this process-local TUI runtime. Its explicit contracts are:

- `AppCommand` or `CoreInput`: user, lifecycle, tick, or completion intent
- `Effect`: work core requests outside itself
- `Completion`: an effect result returning through the same input queue
- `AppEvent`: externally useful transition
- `AppSnapshot` and projections: adapter-facing read models
- `RevisionedPlanningParallelProjection`: revision plus planning/parallel state for the TUI frame
  hot path

Composition owns the opaque `NativeClientRuntime` used by the native shell. It alone assembles the
bounded mailboxes, `CoreEffectRunner`, `CoreRuntime` driver, and application-owned parallel
control-plane handle. The TUI dispatches only `NativeClientEvent` through `dispatch_client_event`
and reads owned snapshots or projections; it cannot name the parallel completion ingress or call a
raw driver/handle. UI-originated inputs remain synchronous so an adapter can bind an accepted
admission before applying an immediate outcome. Core and parallel worker completions return through
private mailboxes and re-enter their owning reducer through `poll_pending_client_event`.

`CoreRuntime`, its effect executor, and its input mailbox are crate-private implementation details,
not a library SDK. Only composition may assemble them. The production callable surface of
`NativeClientRuntime` is closed to event dispatch, completion polling, and owned read-only
snapshots/projections. It exposes no raw runtime, runner, sender, service, mutable state reference,
or borrowed projection. Rust-aware architecture tests pin both visibility boundaries and the exact
facade method ledger, and mutation fixtures prove that reopening a raw type or adding a mutable
facade capability fails `cargo test`.

Every `CoreEffect` variant is structurally paired with one exhaustive dispatch arm. Except for the
two typed local invalidations, an arm must enter exactly one audited completion worker, emit an
immediate `EffectCompleted` input, or use the separately audited turn-terminal worker. Architecture
tests parse the enum, dispatch match, launcher, and runtime trait delegation as Rust syntax, so a
new completion-less arm, wildcard, conditional launcher, or direct TUI worker fails `cargo test`.

The application-owned parallel control plane follows the same totality rule for its seven effects.
Every effect is exhaustively dispatched either through the shared panic-total completion sink or
through its audited synchronous settlement path; wildcard arms, guarded arms, detached workers,
and completion-sink bypasses fail the architecture suite.

Only `CoreRuntime` drives mutable client-runtime state. Adapters must not construct or mutate
`CoreController`, `AppState`, or `TurnStreamState` directly. Effect executors may perform work and
return a completion, but they do not own or mutate runtime state.

`CoreController` is the exhaustive root router, not a bag of per-feature leases. Its private state
is fixed to seven typed slices: `AppState`, startup, session, conversation/turn, read-model loads,
planning, and GitHub review. Each feature reducer owns its generation counters, active
correlations, cancellation flags, and exact-completion checks behind methods; the root only
coordinates cross-feature ordering and projects accepted results into `AppState`. Rust-aware
architecture tests reject additional raw controller fields, visible reducer authority fields,
forbidden reducer dependencies, and silent wildcard handling of conversation stream authority.
`AppState` is physically nested in the controller's private child module. Only the root controller
can name its projection writers; feature reducers return typed local reductions and cannot import
the snapshot store. An AST guard rejects a sibling state module, wider state or writer visibility,
and any visible snapshot storage field.

This establishes one serialized reducer ingress for ClientEvent flow. Retaining the internal
`AppCommand` / `CoreInput` names and not physically enqueueing synchronous UI-originated events do
not create another semantic writer. Global parallel-cleanup state remains owned exclusively by the
application control-plane; the TUI reads its typed owned presentation projection without retaining
or reinjecting a second notice ledger.

The adapter's bounded `BackgroundMessage` lane is presentation-only in production and carries only
operator alerts. Startup, session, conversation, turn, planning, and parallel semantic results
cannot be added to that lane without failing the Rust-aware architecture guard. The same guard
compares every `NativeClientEvent` variant with one unguarded exhaustive dispatch arm, so adding a
new event cannot silently create an unrouted or wildcard-routed state path.

Startup, session loading, conversation selection, turn submission, stream reduction, completion,
and post-turn evaluation use this flow. Parallel mutation remains application-owned but enters the
same client-runtime facade; only composition retains `ParallelModeControlPlaneHandle`. Core may copy
the projection but must not own a second parallel runtime.

The TUI frame hot path calls `revisioned_planning_parallel_projection()` once per terminal
transaction. Its owned `RevisionedPlanningParallelProjection` carries the matching Core revision
and planning/parallel state without materializing an `AppSnapshot`; startup, session-catalog, and
conversation payloads are therefore not cloned for each frame. Full snapshots remain available to
flows that require their broader adapter-facing read model.

`AppState` keeps that full read model as one `Arc<AppSnapshot>` copy-on-write authority.
`CoreDispatchOutcome` and the generic `SnapshotChanged` event share the exact snapshot allocation
for their transition; unchanged, stale, admission-only, and turn-stream inputs therefore clone only
the `Arc`, not the loaded conversation or session catalog. An accepted state mutation uses
`Arc::make_mut`, so a caller retaining an older outcome continues to observe its exact prior
revision. The explicit `snapshot()` pull remains an owned read for consumers that require one.
Snapshot pointer identity and `AppState` revision are not dispatch identities: controller-only and
turn-stream transitions can emit ordered events while sharing the same `AppSnapshot`, so adapters
must still apply every outcome event in order.

Post-turn evaluation also enters Core before changing the planning-worker panel. The command
is admitted only for the latest confirmed completed terminal with no active turn, no already
applied evaluation, and no exact evaluation already in flight. Stale, wrong, and duplicate starts
emit neither an event nor an effect. Core assigns a monotonic correlation over generation, thread,
completed turn, turn workspace, and planning workspace; the effect and completion carry that same
correlation. Only the active correlation may settle, so delayed or duplicate A→B→A completions
cannot clear a newer lease; leaving the admitted conversation/turn lifecycle permanently prunes that
lease even when no intermediate evaluation starts, and a conversation-switch intent cancels it
immediately even if the load must be deferred. Core also checks the execution's outer identity,
nested provenance, optional queue-mutation receipt, and output workspace. Composition converts
malformed worker output into one correlated, redacted failure execution without mutating the shared
continuation gate before Core admission. The continuation gate keeps shared lifecycle generation
separate from request-local worker validity, so a stale timeout cannot cancel a newer request that
captured the same lifecycle generation. Core retains the last exactly accepted
`PlanningWorkerPanelState` as the history seed and resets it when the conversation lifecycle is
invalidated or replaced. The adapter's request field is a neutral compatibility placeholder, not
history authority. After admission, Core clones its own seed, preserves it for an explicit
settlement pause, otherwise selects `RepairRunning` for a protected planning-file change, preserves
it for an empty queue with stop policy, and selects `RefreshRunning` for every remaining case. Core
overwrites the effect request with that accepted state and emits the same state as
`PostTurnEvaluationStarted` before dispatching the asynchronous effect. Only a completion that
passes the active correlation, nested execution identity, and turn-authority checks replaces the
next history seed; stale, duplicate, identity-mismatched, or lifecycle-pruned completions cannot
change it.
The TUI only assigns started and completed projections and no longer seeds evaluation from its
display copy, retains a raw application/planning handle, or calls a planning workspace/runtime use
case directly.

Native TUI startup also owns prompt-log privacy maintenance through this effect path. Production
composition injects a typed maintenance port into `StartupService` but performs no SQLite purge or
clear while building the app. The startup command captures a monotonic generation and the exact
requested workspace; the existing Core startup worker runs maintenance first and then startup
checks for that same workspace. Capture enabled selects retention purge, while capture disabled
selects full clear. A maintenance error or panic becomes a fixed, redacted, non-fatal startup
warning. A startup provider panic becomes one redacted failure completion with the same
correlation. Sensitive Core worker scopes share one permanent delegating panic hook: marked worker
panics emit only a fixed redacted observation, while ordinary panics continue through the prior
hook. Core accepts only the active generation/workspace pair, so stale A→B→A results and
duplicate completions cannot settle a newer request. App construction, `prepare_runtime`, and the
first draw remain eligible while maintenance or the startup probe is blocked. CLI, Admin,
Telegram, and other non-TUI compositions retain their existing synchronous maintenance semantics.

Turn submission admission is core-owned and single-flight. The TUI stages prompt intent without
clearing the editor or appending transcript history, then commits that local projection only after
core emits an accepted admission. An active submission produces an explicit rejection event and no
worker effect, so adapter and core state cannot diverge into a phantom "starting turn".

Runtime stop admission is also core-owned. `:stop` still pauses auto-follow and parallel automation
in the TUI immediately, then sends a correlation-free command. Core assigns a monotonic stop
generation, roots it in the active turn submission when one exists, admits one request at a time,
and drops stale or duplicate completions. Composition serially broadcasts the existing
`request_stop_all_sessions` signal. A successful request made before `TurnStarted` is synchronized
exactly once after the first correlated start; a provider-call error reopens admission, while a
fail-closed interrupt stream notice does not reopen it before an exact retry, terminal, or
conversation transition. The outbound signal remains intentionally global, so Core correlation
governs admission and lifecycle but does not narrow which runtime sessions receive `:stop`.
Provider execution runs outside command dispatch. If a turn or conversation transition invalidates
an in-flight attempt, Core invalidates the worker permit and keeps the stop lease until its exact
completion; new turn admission and a newly requested conversation load wait for that settlement.
This ordering prevents a delayed global broadcast from interrupting work admitted after the stop
request. A provider panic returns as the same correlated failure completion instead of wedging the
lease.

Manual prompt preparation follows the same admission rule. The TUI sends a correlation-free intent
and binds its editor, delivery, and parallel-mode context only after core accepts it. Core assigns
the correlation, permits one preparation at a time, and drops stale or duplicate completions. The
TUI still verifies the accepted workspace and unchanged draft before applying the prepared result.
Cancellation invalidates the background worker permit but retains the physical single-flight lease
until exact settlement. Preparation rechecks that permit after blocking reads and immediately
before bootstrap stage, promotion, and task-intake commit, so an invalidated A→B→A request cannot
overlap a newer mutation. Worker panic is reduced to an exact rejected completion.

Active-turn steering uses the same authority boundary. Core admits only an exact active
submission/thread/turn identity, assigns a correlation rooted in that submission, starts one
provider worker, and drops stale completions. The TUI owns only the confirmation modal and the
editor revision needed to clear an unchanged draft after success. A terminal turn event does not
invalidate an already accepted steer, but a conversation identity transition does.

`ConversationRuntimeSnapshot` is the single process-local authority projection for the active-turn
phase and workspace, approval request/decision/review, auto-follow budget/pause/phase, and post-turn
evaluation/route state, including the accepted planning-handoff provenance. The mutable authority
lives only inside the conversation feature reducer.
`ConversationViewModel` retains one private immutable copy of that snapshot and replaces it
atomically from a Core outcome; it has no parallel set of semantic fields or transition methods.
The adapter installs the outcome snapshot before reducing its presentation events and retains the
prior snapshot only as immutable transition input for terminal facts such as the just-finished turn
workspace or id.

Approval decisions are also core-owned and single-flight. Core admits a decision only for the
current pending approval on the active turn, owns its submitting and submitted states, and prevents
duplicate provider submissions. Approval identity is the full `{approval_id, server_request_id}`
pair: a same-label request from a newer server exchange cannot be cleared by an older resolution,
and a duplicate exact request cannot erase an in-flight decision lease. Composition performs the
provider call and returns its correlated completion. The TUI owns only the approval modal
projection and retry status copy.

Approval review stream updates are persisted through a Core-owned serial effect queue. Each write
is correlated to the exact turn, workspace, thread, and review; composition performs the existing
idempotent Review Center write off the TUI thread. Late failures are shown only while that
conversation identity is still current, so an older write cannot add an error to a new conversation.

GitHub review setup and polling are core-correlated. Before the first delivered frame, the TUI only
parses environment values into `PendingFirstFrame`; it performs no Git, credential, process, or
network discovery. After a draw succeeds and its post-draw size check remains stable, the TUI
dispatches `SetupGithubReviewPolling` exactly once for that frame epoch. Core owns the monotonic
setup generation, exact workspace correlation, identical-request coalescing, target admission,
successful poll cursor, and stale/duplicate completion rules. A newer workspace setup invalidates
the prior service, target, cursor, and in-flight poll, including A→B→A races. Composition performs
branch discovery and service construction off the TUI thread and retains one service only for the
exact accepted setup correlation; a poll cannot use another generation's service. The TUI owns the
poll interval and `PendingFirstFrame` / `Discovering` / `Active` / `Disabled` / `SetupError`
presentation only.

Parallel peek loads are core-correlated reads. Core assigns a monotonically increasing generation
to the requested thread, lets the latest request supersede the previous one, and drops unmatched or
duplicate completions before they reach the TUI. The TUI owns agent selection, loading/status/scroll
presentation, and clears that preview when another shell overlay supersedes it. Peek results never
replace the interactive conversation.

Review Center reads follow the same latest-wins boundary. Core correlates the workspace and optional
active thread, starts the authority read through its effect runner, and drops stale or duplicate
completions. Section-level failures remain data in a core-owned snapshot so one unavailable source
does not hide the other sections. The TUI owns overlay lifecycle and display-only thread context,
and requests a fresh load when the current workspace or thread identity changes.

An accepted post-turn completion updates the core planning-runtime projection in the same
correlated dispatch. Core normalizes the evaluator's pause, budget, stop-rule, and continuation
permit inputs from the current conversation authority rather than trusting a TUI projection. Its
route request is internal to `NativeClientRuntime`: composition checks the Core-owned stop/rearm
gate, offers the exact prompt to the private parallel control-plane at most once, and re-enters Core
with the same correlation and one typed `ParallelConsumed | AutoSubmit | NoContinuation`
resolution. Only the final settled event reaches the TUI. A stale, duplicate, policy-superseded, or
ABA route cannot submit an auto-follow prompt or revive parallel work. TUI conversation state does
not retain a second planning-runtime copy or continuation gate. Disabling parallel routing settles
a parallel-only evaluation, but preserves an exact in-flight evaluation when the independently
configured single-session auto-follow budget can still consume its result.

Session rename is also core-correlated and single-flight. The TUI enters pending state only after
Core returns an accepted admission with the exact correlation; typed active/catalog/conversation
rejections start no provider effect and preserve the editor draft. A successful provider
acknowledgement updates the matching catalog row, loaded conversation title, and stream identity
before the TUI receives the accepted catalog and stream projections. Catalog loads and same-thread
conversation loads requested during that mutation are retained and started after it settles, so an
older read cannot restore the previous title. Core drops provider completions that do not match its
full active correlation. The TUI always applies semantic projections from a completion already
accepted by Core; only settlement of the local editor draft, pending feedback, status, and selected
row requires the exact locally admitted correlation.

## State Authority

| State | Authority |
| --- | --- |
| cursor, modal, overlay, editor buffer, selected row | inbound adapter |
| session/conversation lifecycle, active turn, approval, auto-follow/post-turn, in-flight effects, stream reduction | core |
| parallel wake/effect ordering and stale-completion guards | application control-plane |
| task/direction/queue authority, leases, session records, delivery claims | SQLite-backed stores |
| eligibility, capacity, retry, validation, stale-event decisions | domain |
| delivered frame, viewport/back-buffer trust, host-scrollback receipt, terminal recovery | terminal transaction/adapter |

State that affects an invariant or must survive restart cannot live only in TUI state. Rendering or
focus state should not be promoted to domain authority. Adapter-local presentation state must not
duplicate semantic lifecycle or in-flight-operation authority already owned by Core. Terminal
delivery state describes what the host terminal actually accepted and therefore must not be
inferred from a successful client-state transition alone.

`NativeTuiApp` is a private host aggregate with exactly four typed slices: shell-local presentation,
conversation projection/composer state, planning presentation, and runtime capabilities. It has no
flat semantic fields and exposes no `Deref`, `AsRef`, or mutable aggregate escape. Startup and
conversation snapshots have already passed Core correlation checks when the TUI receives them, so
the adapter projects them directly instead of retaining duplicate startup/conversation pending
gates.

Shell-local startup, session catalog, selection, overlay, and exit-confirmation fields have one
writer: `reduce_shell_chrome`. Core snapshots and domain-derived browser selections enter as typed
projection events; controllers cannot assign those fields directly. The shell reducer module is
crate-private, and a Rust AST guard rejects field assignments, mutable borrows, whole-state
replacement outside the dispatch seam, and non-exhaustive event routing.

Production terminal/frontend code cannot borrow `NativeTuiApp` from `ShellRuntime`. The runtime
returns owned terminal-sync projections and `InlineShellFrameModel` values, then accepts only named,
narrow mutations for render receipts, transcript-handoff acknowledgement, and queue hit-area
cleanup. `app()` and `app_mut()` remain test-only fixtures. Architecture tests pin the four-slice
field ledger, reject aggregate-reference and trait escape hatches, and reject reintroduction of the
retired duplicate correlation gates.

## Planning Boundary

```text
inbound adapter
  -> application/service/planning
  -> application/port
  -> adapter/outbound/{db,filesystem,app_server}
```

The private SQLite store is authoritative. Planning workspace files are operator-editable details,
prompts, staged drafts, exports, and recovery evidence. Accepted changes use revision-aware
validation and mutation; hidden worker output never writes SQL or protected planning files directly.

Planning setup, Planning Doctor, and reset-failure recovery share the existing Core planning-runtime
refresh effect. One application inspection use case reads one coherent aggregate workspace record
for either present or absent state, then returns one runtime projection together with a Core-owned
doctor snapshot; the TUI does not perform a second workspace inspection. Core owns generation and
active-correlation state in one planning-runtime coordinator. Only the exact correlated effect
completion can settle that coordinator; post-turn and generic projection writers cannot impersonate
an init, doctor, or reset-recovery result. A newer same-workspace writer explicitly supersedes the
in-flight read and schedules a replacement inspection, so the old successful read cannot overwrite
the writer projection or complete the operation. The TUI keeps typed
`Idle | Loading | Ready | Failed` presentation state and the exact init, doctor, or reset-recovery
operation plus a presentation revision captured when that operation starts. Rebinding a replacement
inspection changes only its correlation and preserves that initial revision. Stale, duplicate, ABA,
closed-overlay, workspace-drifted, or newer-UI-intent completions cannot choose a setup branch or
replace status. Inspection failures remain distinct from an absent workspace, and reset recovery
preserves both the reset error and any inspection error.

TUI reset, simple-draft stage/load/promotion, and planning/directions editor staging, save, and
promotion share one Core-owned planning-workspace operation coordinator. Core assigns a monotonic
correlation containing the exact workspace and operation kind, plus reset target,
simple-draft/session identity, editor-stage target, or editor-mutation identity where applicable.
An editor-mutation identity includes action, planning/directions target, draft, full source editor
session, session-local buffer revision, and the source planning revision for direction-detail or
queue-idle maintenance drafts. Editor-stage targets distinguish planning manual,
direction detail with its exact direction id, and queue-idle prompt. Repeating that exact operation
coalesces without starting another worker; any different operation is reported as busy.
Composition executes provider work off the TUI input thread and converts panics into exact
correlated error completions. These workers use the shared redacting panic boundary, so a provider
panic payload is not emitted to stderr. Only that exact completion settles the coordinator. Core rejects a
reset target, editor-stage target/session or empty draft, editor load session, or promotion draft
that does not match the correlation. Before provider I/O, the runner rejects an editor mutation
whose operation, workspace, action, target, draft, source session, or buffer revision differs from
its request, then invokes exactly one existing save or promote use case. Stale, duplicate,
workspace-drifted, target-drifted, wrong-kind, and ABA completions cannot present a result.

An accepted simple stage creates a session identity from its stage generation; an accepted editor
load replaces it with a load-generation identity. The TUI keeps those identities with review/editor
state so a late load cannot replace a newer editor and a late promotion cannot close a newer
overlay. Editor file bodies travel only in typed request/completion payloads and are redacted from
debug output; they never enter correlation data. Planning manual staging uses the existing planning
`Loading` step; directions staging uses a distinct `EditorLoading` step that preserves the summary,
selection, and pending direction. Only an exact completion for the still-current presentation opens
the editor. Failure returns planning manual staging to detail selection, direction-detail staging to
its confirmation, and queue-idle staging to directions overview. A coalesced retry rebinds the
current presentation revision. Closing, workspace drift, or an approval overlay can settle the
background operation without opening an editor behind newer UI. For an exact reset or promotion completion whose
workspace is current, the TUI always pauses post-turn continuation and refreshes the Core runtime
projection, including after an A-to-B-to-A workspace round trip. A separate presentation revision
gates status and overlay changes: a newer UI intent keeps its copy while authority/runtime
reconciliation still runs. Closing or changing an overlay does not cancel destructive work.
Editor buffer revisions start at zero for each session and advance only after an actual body
mutation. Save keeps the editor interactive and never pauses continuation or refreshes runtime
authority: an exact completion updates validation and clears dirty state, while a completion for an
older revision may update validation but preserves the newer body and dirty state. Promote also
keeps editing available while in flight. A maintenance promotion must still match its staged
planning revision and its hidden active-file baselines before any active write. It writes only the
files exposed by that editor, preserves hidden result output in the authority document commit,
commits active documents and structured authority through one SQLite revision transaction, and
returns the committed revision so newer in-flight edits can promote again without conflicting with
their own previous commit. Production uses that private authority for Git workspaces (and for all
workspaces on Windows). Legacy plain Unix workspaces stay file-backed so existing files are not
silently replaced without an adoption migration; revision-bound maintenance staging fails closed
there before it mutates accepted authority. In the current workspace every exact promote completion
pauses continuation and refreshes runtime authority, even if presentation has since changed; only a
positive success for the exact current target, session, and buffer revision closes planning or
returns directions maintenance to overview. Zero-count success keeps the editor open after
reconciling the saved draft, and errors or stale presentation/session/revision completions never
close or replace editor state. All four planning/directions save/promote entry points dispatch the
typed Core mutation command. Production TUI code exposes no direct planning workspace use-case
handle, and Rust-aware architecture guards reject direct method, UFCS, path, function-pointer, and
macro callable forms. This remains one coordinator and worker path, not a deferred queue, actor, or
cancellation abstraction.

Opening the TUI Queue overlay first applies shell chrome, then dispatches a core load command. Core
assigns a monotonically increasing generation to the workspace and active-thread identity, lets the
latest request supersede the previous one, and drops stale or duplicate completions. Composition
executes the coherent application read and maps its runtime projection, planning revision, tasks,
and typed failure into a core-owned snapshot. Presentation reads only the resulting immutable screen
model: loading and failed states remain read-only, while a ready state binds visible rows, selection,
revision, and destructive-action tokens to one snapshot. Workspace, thread, or visible-revision drift
starts a new correlated load; redraw and resize paths perform no authority I/O.

TUI queue remove and undo intents enter through a core command. Core assigns a monotonically
increasing generation to the workspace, active-thread identity, base planning revision, and exact
task status/update tokens, admits only one mutation at a time, and drops stale, duplicate, or ABA
completions. Composition maps that core-owned intent into `PlanningQueueUseCases`, whose
cancellation transaction submits the request and performs a coherent runtime and queue-authority
readback after both success and failure. The TUI controller owns only remove/undo presentation and
projects settlement from the accepted correlation, including its mutation kind and captured
receipt; it does not schedule the worker or mint operation IDs. Only the exact accepted correlation
in the same workspace/thread context may reconcile returned authority into the projection. Closing
the Queue overlay resets local selection and feedback but does not discard the accepted mutation,
and duplicate destructive intents remain blocked until Core consumes its completion.

This is process-local Core correlation, not store-wide idempotency. Generations are not persisted,
do not yet carry repository incarnation, and do not promise exactly-once execution across process
restart; durable revision and task-token validation remain the application/store boundary's guard.

`AKRA_HOME` must resolve to a trusted absolute root. SQLite database and sidecar files are private,
regular, single-link files. Repository identity is bound to a private incarnation marker so a
reused checkout path cannot silently adopt an earlier repository's authority.

## Parallel Boundary

```text
TUI intent
  -> NativeClientEvent
  -> composition-private application control-plane handle
  -> domain decision
  -> durable store / effect runner
  -> private completion mailbox
  -> exact control-plane reduction / projection
  -> core snapshot / TUI rendering
```

The current control-plane is a mutex-serialized synchronous facade. It provides one application
writer, effect accounting, stale-completion dropping, wake coalescing, durable backpressure, and a
single projection source. Do not add a mailbox actor, raw parallel service owner in TUI/core, or a
second dispatch queue without revisiting that decision.

Every asynchronous control-plane launcher uses one panic-redacting completion worker. Success,
ordinary failure, inactive-epoch rejection, and panic each produce exactly one terminal completion
with the original workspace, epoch, and effect identity. Only that exact identity may clear the
in-flight ledger; stale, duplicate, and ABA completions are presentation no-ops. The outer parallel
agent worker follows the same rule and converts an unexpected panic into one redacted
`StreamFailed` worker event. A failed wake or tick invalidates the possibly mutated projection and
must complete an exact supervisor refresh before dispatch can resume. A failed first entry closes
its epoch and cancels partial durable dispatch state; a failed re-entry preserves the already-on
mode but still requires a fresh projection.

Periodic pending-dispatch polling is admitted by that facade but reads durable authority only in
the effect runner. The runtime assigns a monotonic operation correlated to the exact workspace and
epoch, keeps one poll in flight, and coalesces later ticks. Only the matching completion may start a
wake or follow-up tick; disable, workspace switch, duplicate, and ABA completions are dropped.
Read failures preserve the current projection and the next scheduled tick remains a retry.

Post-turn continuation reuses the queue-head fact from the exact runtime projection accepted with
the completed evaluation; it does not reread planning authority while holding the facade. Durable
dispatch enqueue and cancellation also run only in effect-runner workers. The runtime serializes
them with monotonic operation/workspace/epoch correlation, preserves refresh then queued-wake then
pending-poll priority, and coalesces enqueue intent by workspace, epoch, and canonical durable
trigger even while its first worker is running. Slot-capacity and task-intake requests therefore
share one durable task-intake mutation. Cancellation runs for the exact closed epoch ahead of
queued enqueue work. A current cancellation failure is shown in workspace status. Any failed
cleanup enters a bounded unsettled-cleanup ledger with its original operation, workspace, epoch,
command identity, and error. This application-owned ledger is the only authority for unsettled
cleanup state. Its typed owned presentation projection retains the matching global runtime notice
independently of conversation Loading/Failed state, so a later Ready conversation surfaces it
without replacing that conversation's workspace projection. The TUI does not copy the ledger into
adapter or conversation state; global-notice presentation events only invalidate the current frame
for redraw. Retrying the exact correlation schedules the original cancellation through the
control-plane effect runner. The production TUI control-plane pulse
retries the oldest unsettled exact correlation at its existing bounded interval, independent of
conversation Loading/Failed state. Active replacement-epoch work keeps refresh, queued wake, then
pending-poll priority; one bounded deferral gives cleanup the next idle pulse before another normal
cycle, so neither lane starves. Cleanup cadence is independent of pending-poll cadence, and mutation
coalescing keeps the retry single-flight. Success settles the ledger entry and clears the notice.
Other late or ABA completions remain diagnostic-only.

Pool mutations also take a repository-scoped OS lock. Every allocation gets an unguessable exact
generation carried through leases, sessions, events, delivery, and cleanup; delayed events compare
that generation before mutation. SQLite remains authoritative on every supported platform.

Supervisor inspection projects bounded session detail for every live pool lane instead of only the
currently selected session. Each detail row remains keyed by exact slot/session identity and is
sorted and deduplicated before presentation. Optional agent profile lookup adds display name and
role to the roster at the application boundary; lookup failure leaves those fields unknown. The
TUI joins pool, roster, session detail, and distributor state by those identities and surfaces a
disagreement as `DESYNC`; rendering never reads Git, GitHub, SQLite, or profile storage.

Post-turn mutation captures continuation and parallel-epoch permits. Long work stays outside their
bounded commit sections; an invalidated permit may still produce diagnostics but cannot change task
authority or enqueue delivery.

## Delivery and Process Boundaries

Parallel acquisition freezes the configured push remote, credential-free canonical GitHub URL,
repository/visibility, integration branch, fetched base, source SHA, and ordered reviewed range.
Remote or review drift fails before the corresponding mutation.

Host-owned Git operations:

- pin trusted native executables and clear inherited Git execution/routing controls
- disable hooks, fsmonitor, signing, replacement objects, lazy fetch, prompts, and external diff
- audit effective repository/worktree configuration by key name before mutation
- reject executable filters/drivers, unsafe worktree redirection, alternate refs, external tools,
  special paths, oversized changes, unmerged entries, and hidden dirty submodule state
- use isolated credential-free network configuration and a separately verified API identity

Subprocesses use `src/subprocess.rs`. Linux uses process-group plus bounded `/proc` descendant
containment, Windows uses a kill-on-close Job, and other Unix targets guarantee only process-group
containment. Same-user hostile racing remains outside the filesystem-only threat boundary.

## TUI Ownership

TUI changes must keep state/reducer, controller/effect, projection/copy, theme/chrome,
rendering/layout, and terminal-adapter responsibilities separate. Visual tokens belong behind
`AkraTheme`; append-only rows split across host scrollback and live viewport cannot insert panel
chrome into the stream.

Every actual shell-overlay identity change is emitted by the reducer as one typed
`ShellOverlayTransition` containing its `from`, `to`, and exit mode. The root TUI coordinator
installs the reduced state and then sends that transition to the sole overlay-cleanup owner, whose
matches enumerate every `ShellOverlay` and both `Suspend`/`Exit` modes without a wildcard.
Entering `Approval` suspends cleanup and preserves the departed overlay's local state.
`DirectionsMaintenance` is the one overlay restored after approval closes; other approval
interruptions return to hidden chrome without erasing their in-flight local state. Explicit close
only dispatches `OverlayClosed`; all exit cleanup is derived once from the returned transition.

The inline conversation tail path combines adapter-local UI state with one owned
`RevisionedPlanningParallelProjection` from `revisioned_planning_parallel_projection()` into an
immutable `ConversationScreenModel`. The terminal transaction captures that narrow Core projection
once; it does not clone the broader `AppSnapshot` or its startup, session-catalog, and conversation
payloads for a frame. A single owned tail projection derived from the screen model is compared by
the redraw cache and then painted by the terminal transaction. Tail status, planning, parallel,
queue, GitHub, transcript, layout, animation, and prompt-focus helpers do not receive
`NativeTuiApp` or application service handles. Conversation semantic state stores messages, not
cached Ratatui `Line` values.

The same transaction captures parallel mode, in-flight effect, supervisor inspection, withheld
reason, the typed owned unsettled-cleanup notice projection, and event-stream facts once. The
`ConversationScreenModel` combines those facts without mutating conversation state, and pure draw
consumes the result. Supersession row planning, host-scrollback/live-tail splitting, prompt lock,
animation, and drawing consume that immutable projection instead of reacquiring the control-plane
mutex or sampling another clock. High-frequency prompt, pulse, and scheduler checks share a
panel-only projection and do not clone transcript or event-stream rows.

Focused Supersession is a full inline-main-buffer inspection, not an alternate-screen TUI. It hides
and locks the composer, preserves its draft, and renders one responsive 16-row Parallel Operations
view. Hidden Supersession while parallel mode remains enabled is passive and leaves the composer
available. Lane selection stores stable slot/session identities and resolves them against each new
snapshot, so refresh reorder cannot silently move focus to another worker. The selected lane
separates typed commit, validation, PR, review, integration, remote-verification, and cleanup gates;
absence of an owned fact remains `unknown`.

Before `Terminal::draw`, the transaction combines the conversation projection and exactly one
active overlay into an owned `InlineShellFrameModel`. Its `InlineInspectionFrameModel` variant owns
the view, widget-local state, geometry-dependent scroll decisions, and expected feedback baseline.
The capture boundary may read UI-local state but cannot reacquire Core, application services, the
parallel control plane, or outbound I/O. Production `shell_rendering.rs` and
`shell_rendering/**` consume only this owned frame model, mutate only Ratatui's `Frame`, and return
an `InlineFrameRenderReceipt`; they cannot receive `NativeTuiApp`, dispatch commands, sample
clocks, or retain adapter state.

The terminal transaction commits a render receipt only after the draw and post-draw terminal-size
checks succeed. Its exact attempt gate discards failed, resize-raced, stale, and duplicate
receipts. Receipt application compares the captured baseline before applying activity, editor,
help, approval, session-list, or queue-hit-area feedback, so an older frame cannot overwrite a
newer UI edit.

Activity frame capture joins retained progressive payloads with the Core-published item-lifecycle
snapshot through exact item identity and authoritative consistency records. The resulting owned
frame carries typed outcome, summary, elapsed time, and wait reason; presentation must not infer
these facts from prose. The adapter keeps only weak references to both immutable snapshots, and the
render receipt owns row hit areas so keyboard and mouse folding update the same bounded expansion
state without making rendering an input authority.

Session frame capture creates one owned `SessionOverlayScreenModel` before presentation. The model
combines the Core-published catalog projection with workspace, committed and edited query, project
filter, one page projection, stable selected thread identity, page-local selected index, rename
editor state, warnings, and key availability. Page projection and selection repair run once;
list/detail/warning/key builders and renderers cannot reread `NativeTuiApp` or services. Capture
builds the owned overlay view and frame-local Ratatui `ListState`; only a stable render receipt may
compare-and-apply the resulting state. Repeated redraw and resize do not dispatch catalog work or
call services.

Core is the sole session-catalog admission and correlation authority. The adapter emits typed
ensure-loaded or explicit-refresh intent without writing `Loading` or suppressing duplicates from
its display copy. Core applies settled-state policy, coalesces the exact in-flight workspace and
limit, and publishes every accepted `Loading` transition with a full generation/workspace/limit
correlation. `SessionState` is only the adapter projection of those Core events. A Core-accepted
rename always applies its semantic catalog and active-stream projection; an exact local pending
receipt controls only editor, feedback, selection, and status settlement.

The auto-follow turn-budget overlay keeps only an active, uncommitted edit draft. When the editor
is closed, status and review presentation read the canonical policy from the Core conversation
runtime snapshot; the adapter does not retain or reverse-sync a second budget value. Manual and
auto-follow turn submission are admitted against that same snapshot. An automatic submission must
consume the exact settled post-turn correlation once; an uncorrelated, stale, duplicate, paused, or
budget-ineligible request is rejected before a provider effect starts.

Planning-worker diagnostics retain the domain `PlanningWorkerPanelState` inside a sealed,
read-only Core projection from the Core-started post-turn event through asynchronous completion and
the screen model. Only accepted Core start/completion or conversation `Idle | Loading` lifecycle
events can replace/reset that projection; draft/session UI intent cannot clear it optimistically.
TUI presentation derives labels and content visibility without an adapter-owned status DTO or
round-trip mapper.

The detailed, test-guarded contract is
[TUI Layered Architecture](../design/07-tui-layered-architecture-and-aesthetic-contract.md).

## Forbidden Directions and Gates

- `domain` must not import application, core, adapters, runtime, or IO frameworks.
- `application` must not import core, TUI, HTTP, Telegram, or concrete outbound adapters.
- `core` must not import inbound UI/transport types or concrete outbound adapters.
- TUI must not bypass core/application command gates for planning or parallel mutation.
- Outbound adapters must not encode domain policy.

Verify boundary-sensitive changes with:

```bash
. "$HOME/.cargo/env"
cargo test --test architecture_boundaries
cargo fmt --all -- --check
cargo test --locked
```

Run `bash scripts/check_native_pr.sh` for broad native/TUI changes.
