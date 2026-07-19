# Runtime Architecture

[한국어](../ko/reference/architecture.md)

This is the canonical architecture and authority reference for the shipped runtime.

```text
adapter/inbound -> core or application -> domain
core -> application -> domain
application -> outbound ports -> adapter/outbound
composition -> concrete wiring
```

## Layer Ownership

| Layer | Owns | Must not own |
| --- | --- | --- |
| `adapter/inbound` | input mapping, rendering, local focus/editor/selection state | domain policy, durable task truth, dispatch policy |
| `core` | app command/event/effect/completion flow, app state, projections, snapshots | TUI/HTTP/Telegram types or concrete DB/Git/filesystem adapters |
| `application/service` | use-case orchestration, ordering gates, transactions, control-plane handles | widgets, terminal events, transport DTOs |
| `application/port` | outbound contracts required by application services | concrete integration details |
| `domain` | pure invariants, validation, decisions, state transitions | async runtime, IO, logging, UI, database, filesystem, or Git calls |
| `adapter/outbound` | app-server, DB, filesystem, Git, GitHub, and Telegram integration | business policy |
| `composition` | dependency construction and concrete wiring | domain decisions |

Mapping stays in adapters. Policy stays in domain or application services. Add a port only for a
real outbound boundary.

## Core Runtime

`src/core` is a headless app runtime, not a replacement for application or domain layers. Its
explicit contracts are:

- `AppCommand` or `CoreInput`: user, lifecycle, tick, or completion intent
- `Effect`: work core requests outside itself
- `Completion`: an effect result returning through the same input queue
- `AppEvent`: externally useful transition
- `AppSnapshot` and projections: adapter-facing read models

Startup, session loading, conversation selection, turn submission, stream reduction, completion,
and post-turn evaluation use this flow. Parallel mutation remains application-owned and enters
through `ParallelModeControlPlaneHandle`; core may copy the projection but must not own a second
parallel runtime.

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

Approval decisions are also core-owned and single-flight. Core admits a decision only for the
current pending approval on the active turn, owns its submitting and submitted states, and prevents
duplicate provider submissions. Composition performs the provider call and returns its correlated
completion. The TUI owns only the approval modal projection and retry status copy.

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
correlated dispatch. TUI conversation state does not retain a second planning-runtime copy.

Session rename is also core-correlated and single-flight. A successful provider acknowledgement
updates the matching catalog row, loaded conversation title, and stream identity before the TUI
receives the accepted catalog and stream projections. Catalog loads and same-thread conversation
loads requested during that mutation are retained and started after it settles, so an older read
cannot restore the previous title. The TUI maps those core projections and owns only the rename
editor draft, pending feedback, and selected row.

## State Authority

| State | Authority |
| --- | --- |
| cursor, modal, overlay, editor buffer, selected row | inbound adapter |
| session/conversation lifecycle, in-flight effects, stream reduction | core |
| parallel wake/effect ordering and stale-completion guards | application control-plane |
| task/direction/queue authority, leases, session records, delivery claims | SQLite-backed stores |
| eligibility, capacity, retry, validation, stale-event decisions | domain |

State that affects an invariant or must survive restart cannot live only in TUI state. Rendering or
focus state should not be promoted to domain authority.

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
session, and session-local buffer revision. Editor-stage targets distinguish planning manual,
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
keeps editing available while in flight. In the current workspace every exact promote completion
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
  -> application control-plane handle
  -> domain decision
  -> durable store / effect runner
  -> projection
  -> core snapshot / TUI rendering
```

The current control-plane is a mutex-serialized synchronous facade. It provides one application
writer, effect accounting, stale-completion dropping, wake coalescing, durable backpressure, and a
single projection source. Do not add a mailbox actor, raw parallel service owner in TUI/core, or a
second dispatch queue without revisiting that decision.

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
and command identity. Its matching global runtime notice is retained independently of conversation
Loading/Failed state, so a later Ready conversation surfaces it without replacing that
conversation's workspace projection. Retrying the exact correlation schedules the original
cancellation through the control-plane effect runner. The production TUI control-plane pulse
retries the oldest unsettled exact correlation at its existing bounded interval, independent of
conversation Loading/Failed state. Active replacement-epoch work keeps refresh, queued wake, then
pending-poll priority; one bounded deferral gives cleanup the next idle pulse before another normal
cycle, so neither lane starves. Cleanup cadence is independent of pending-poll cadence, and mutation
coalescing keeps the retry single-flight. Success settles the ledger entry and clears the notice.
Other late or ABA completions remain diagnostic-only.

Pool mutations also take a repository-scoped OS lock. Every allocation gets an unguessable exact
generation carried through leases, sessions, events, delivery, and cleanup; delayed events compare
that generation before mutation. SQLite remains authoritative on every supported platform.

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

The inline conversation tail path projects one core `AppSnapshot` plus adapter-local UI state into
an immutable `ConversationScreenModel`. A single owned tail projection derived from it is compared
by the redraw cache and then painted by the terminal transaction. Tail status, planning, parallel,
queue, GitHub, transcript, layout, animation, and prompt-focus helpers do not receive
`NativeTuiApp` or application service handles. Conversation semantic state stores messages, not
cached Ratatui `Line` values.

The same transaction captures parallel mode, in-flight effect, supervisor inspection, withheld
reason, and event-stream facts once. Supersession row planning, host-scrollback/live-tail splitting,
prompt lock, animation, and drawing consume that immutable projection instead of reacquiring the
control-plane mutex or sampling another clock. High-frequency prompt, pulse, and scheduler checks
share a panel-only projection and do not clone transcript or event-stream rows.

The auto-follow turn-budget overlay keeps only an active, uncommitted edit draft. When the editor
is closed, status and review presentation read the canonical policy from the conversation model;
the adapter does not retain or reverse-sync a second budget value.

Planning-worker diagnostics retain the domain `PlanningWorkerPanelState` directly from post-turn
execution through the screen model. TUI presentation derives labels and content visibility without
an adapter-owned status DTO or round-trip mapper.

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
