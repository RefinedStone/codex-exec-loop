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

Opening the TUI Queue overlay first applies shell chrome, then a Queue-specific controller loads the
paired runtime projection and queue authority on a background thread. The result carries a request
ID plus workspace and active-thread identity. Presentation reads only the resulting immutable screen
model: loading and failed states remain read-only, while a ready state binds visible rows, selection,
revision, and destructive-action tokens to one snapshot. A stale request or workspace, thread, or
visible-revision drift is ignored and starts a new correlated load; redraw and resize paths perform
no authority I/O.

TUI queue remove and undo intents open one overlay-independent pending gate. Its adapter correlation
envelope carries an operation ID, workspace and active-thread identity, plus the cancellation
request's base planning revision and exact task status/update tokens. The controller does not mutate
the visible projection optimistically or call the cancellation service on the terminal input path.
`PlanningQueueUseCases` owns the cancellation transaction: it submits the request, then performs a
coherent runtime and queue-authority readback after both success and failure. The TUI controller owns
background scheduling plus operation/context correlation and settlement; only the exact pending
operation in the same workspace/thread context may reconcile the returned authority into the
projection. Closing the Queue overlay resets its local selection and feedback but does not discard
the pending operation, and duplicate destructive intents remain blocked until its completion is
consumed.

This is TUI settlement correlation, not store-wide idempotency. Operation IDs are not persisted,
do not yet carry repository incarnation, and do not promise exactly-once execution across process
restart; durable revision and task-token validation remains the application/store boundary's guard.

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

The auto-follow turn-budget overlay keeps only an active, uncommitted edit draft. When the editor
is closed, status and review presentation read the canonical policy from the conversation model;
the adapter does not retain or reverse-sync a second budget value.

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
