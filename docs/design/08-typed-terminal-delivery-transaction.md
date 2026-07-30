# Typed Terminal Delivery Transaction

Status: **Accepted for implementation; not yet shipped**

Decision date: 2026-07-30

Target: native inline TUI, initially the parallel supervisor event stream

Related contracts:

- [Runtime Architecture](../reference/architecture.md)
- [TUI Layered Architecture And Aesthetic Contract](07-tui-layered-architecture-and-aesthetic-contract.md)
- [Terminal UI Testing Methodology](../validation/terminal-ui-testing-methodology.md)
- [TUI Coverage Matrix](../validation/tui-coverage-matrix.md)

## Decision Summary

Akra will implement a narrow terminal-delivery state machine inside the inbound TUI adapter.
The first consumer is the parallel supervisor event stream.

The change will:

1. retain stable event identity through projection and terminal delivery;
2. replace separate live and scrollback event stores with one canonical event window;
3. replace rendered-line comparison with a monotonic terminal delivery frontier;
4. partition durable and live events once into disjoint typed models;
5. commit host-scrollback delivery separately from frame-render feedback;
6. represent ambiguous terminal writes explicitly instead of blindly replaying them; and
7. make unsafe renderer and terminal paths fail architecture tests.

This decision activates only the narrow extraction previously described as the conditional
terminal-transaction option. It does **not** replace Ratatui or Crossterm, introduce custom
scrollback, move terminal state into Core, or create a new business/application layer.

Until the implementation completion criteria in this document pass, the current terminal adapter
remains the shipped implementation.

## Context

The shipped architecture already separates three kinds of truth:

```text
Core / Application authority
  -> immutable screen projection
  -> terminal delivery transaction
```

The semantic boundaries are substantially enforced:

- Core and the application control plane own lifecycle and mutation authority.
- `ConversationScreenModel` and owned frame models isolate rendering from runtime capabilities.
- production renderers cannot borrow `NativeTuiApp`, call services, or mutate semantic state;
- transcript handoff and frame feedback use exact typed receipts; and
- terminal delivery owns viewport, back-buffer, scrollback, and recovery facts.

The parallel event stream does not yet preserve that contract end to end. Runtime feed entries
arrive with an authority sequence, but the TUI lowers them to rendered `Line` values before
terminal delivery. It then keeps separate live and scrollback collections and uses rendered-line
prefix or overlap comparison as the delivery baseline.

Consequently, copy, localization, wrapping, viewport height, stream retention, or layout changes
can influence whether terminal delivery considers an event new. This is a presentation concern
leaking into side-effect identity.

## Trigger Evidence

This decision uses the four axes required by the terminal validation contract.

### Bug-class recurrence across compatibility boundaries

At base commit `bde8b96c5`, the terminal-related paths had 45 commits since 2026-07-01, including
20 `fix` commits. Not every commit represents the same defect, but the repeated fixes cluster
around the same boundary:

- resize reconciliation;
- host-tail reflow;
- scrollback retention;
- transcript handoff;
- draw-time resize ABA;
- focus reacquisition;
- frame rebuild and invalidation; and
- parallel live-tail continuity.

The observed 2026-07-30 parallel stream failure showed one authoritative runtime event twice in
the user-visible terminal history. The authority database contained one update, so the duplicate
was introduced after semantic reduction, in the projection-to-terminal-delivery path.

This establishes a recurring terminal delivery bug class. It does not by itself prove which
individual terminal primitive produced the duplicate, so implementation must begin with the exact
temporal reproducer.

### Fallback masking risk

Existing tests for parallel runtime-feed priming, scrollback delta insertion, and live-tail
continuity pass. The real TUI still displayed a duplicate when a long GitHub identity error,
wrapping, asynchronous board refresh, and the host/live boundary interacted.

Passing short-marker and fixed-geometry tests therefore does not prove the real transaction.
Line-based baselines can also make a fallback path appear correct while a terminal emulator
interprets insertion, clear, or viewport movement differently.

### Future test-growth cost

Under the current representation, changes to any of these facts can require another temporal
regression:

- status or error text length;
- localization;
- panel or footer height;
- event retention;
- title visibility;
- viewport geometry;
- focus and overlay transitions; or
- insertion strategy.

Tests must continue to cover those combinations, but identity and delivery safety should not
depend on exhaustively enumerating layout permutations.

### Maintainability cost

The current transaction:

- stores the same event in live and scrollback collections;
- discards stable source identity when producing `Line`;
- keeps conversation and parallel rendered-line baselines in one history flush state;
- swaps those baselines to reuse content-diff logic;
- computes the host/live boundary from rendered content; and
- permits architecture guards to pin a line-based split without proving delivery identity.

That structure makes a locally reasonable copy or layout edit capable of changing terminal
side-effect behavior. A stable-ID delivery frontier is smaller and easier to review than continued
content-diff exceptions.

## Scope And Non-goals

### In scope

- parallel supervisor event ingestion and identity;
- one process-local canonical event window;
- owned parallel stream projection;
- host-scrollback delivery planning;
- terminal delivery cursor and receipt;
- live-frame partitioning;
- terminal write uncertainty and recovery;
- renderer input narrowing;
- AST architecture guards;
- deterministic transaction tests, vt100 proof, and required real-terminal captures.

### Not in scope

- changing parallel task, queue, dispatch, lease, or delivery business authority;
- moving terminal delivery state into `ClientState`, Core, domain, or application services;
- adding `CoreCommand`, `Effect`, or a background worker for terminal writes;
- replacing Ratatui or Crossterm;
- implementing custom scrollback;
- generalizing transcript and parallel delivery into a universal framework before the parallel
  contract is proven;
- renaming every existing terminal module; or
- converting every historical supervisor message into a perfect semantic enum in the first slice.

## Authority Model

| Fact | Sole owner | Must not own it |
| --- | --- | --- |
| dispatch, queue, lease, session, and planning truth | application control plane and SQLite-backed authority | TUI event stream, renderer, terminal delivery |
| process-local client lifecycle and accepted correlations | Core | renderer, terminal delivery |
| ordered operator event-stream projection | inbound TUI `ParallelEventStreamState` | Core business state, renderer |
| what the current terminal surface accepted into host scrollback | terminal `ParallelDeliveryState` | Core, application, screen model |
| current live event rows | immutable `ParallelLiveStreamModel` | mutable TUI or terminal state |
| terminal cells | Ratatui renderer and backend | semantic authority |

Terminal history is a projection, not a source of semantic truth. A delivery frontier may suppress
re-delivery to the same terminal surface, but it must never suppress upstream event ingestion or
feed facts back into Core or the application control plane.

## Required Invariants

### I1. Stable identity survives rendering

Every accepted event receives a process-local stream identity that is independent of its text,
style, language, or wrapped row count.

```rust
struct ParallelStreamEventId {
    stream_generation: u64,
    ordinal: u64,
}
```

The source correlation remains available for ingestion deduplication, but terminal delivery uses
the stream event ID.

### I2. One canonical event window

The stream owns one bounded ordered collection.

```rust
struct ParallelEventStreamWindow {
    stream_generation: u64,
    first_ordinal: u64,
    events: Arc<[ParallelStreamEvent]>,
}
```

There must not be independently mutable live and scrollback event collections. Live, durable, and
inspection views are derived from the same immutable window.

### I3. Monotonic delivery frontier

The terminal owns a frontier per stream and terminal-surface generation.

```rust
struct ParallelDeliveryCursor {
    stream_generation: u64,
    delivered_through: Option<u64>,
}
```

The frontier may advance after an exact committed write. It may never move backward because of a
resize, focus change, overlay transition, text change, or viewport growth.

### I4. Durable and live partitions are disjoint

One pure planner creates the host batch and live model from:

- one event window;
- one delivery cursor;
- one exact terminal geometry snapshot; and
- one owned layout input.

The renderer receives only the live model. It cannot receive or reconstruct the durable batch.

For a successful stable transaction:

```text
host_event_ids ∩ live_event_ids = ∅
host_event_ids ∪ live_event_ids = visible retained stream coverage
```

### I5. Copy is not correlation

No parallel delivery decision may use rendered `Line` equality, localized copy, style, or wrapping
as event identity. Copy may change layout and the desired future boundary; it cannot resurrect a
committed event.

### I6. Host and frame receipts are separate

Host-scrollback insertion and frame drawing have different commit points.

- `ParallelHostDeliveryReceipt` proves a specific event range was accepted by the insertion
  operation.
- `InlineFrameRenderReceipt` proves a captured frame was drawn stably and may compare-and-apply UI
  feedback.

A frame draw failure after a successful host insertion must invalidate and retry only the frame.
It must not replay the host batch.

### I7. Exact stale, duplicate, and ABA rejection

A host delivery token binds:

- terminal surface generation;
- stream generation;
- expected delivery frontier;
- proposed next frontier; and
- delivery attempt ID.

Only the active exact token may settle. Duplicate settlement is idempotent. A token from an older
stream, terminal surface, or expected frontier is rejected without mutation.

### I8. Retention gaps are explicit

If the bounded stream drops events that the terminal has not delivered, the planner must produce
one typed gap record. It must not silently advance the cursor or replay unrelated text as overlap
evidence.

### I9. Ambiguous writes do not blindly replay

Physical terminal output is not an atomic database transaction. A backend error can be ambiguous
after a primitive has started. The terminal state therefore distinguishes:

```rust
enum ParallelDeliveryState {
    Ready { cursor: ParallelDeliveryCursor },
    Writing { token: ParallelHostDeliveryToken },
    Uncertain { token: ParallelHostDeliveryToken },
}
```

- failure before a primitive starts is retryable;
- complete insertion commits the receipt even if a later frame draw fails;
- incomplete or ambiguous insertion enters `Uncertain`;
- `Uncertain` never automatically resends the same range; and
- recovery retains the canonical event window and exposes a clear operator-visible recovery
  status.

The product guarantee is exact in successful transactions and fail-closed in ambiguous terminal
I/O. The implementation must not claim impossible process-crash-level exactly-once semantics for
an unacknowledged physical terminal.

## Event Ingestion

Different event sources require different duplicate policy.

### Authority runtime events

Use the exact workspace identity and authority sequence:

```text
Authority { workspace, sequence }
```

Stale or duplicate sequences do not append. An accepted newer event receives the next stream
ordinal.

### Local accepted UI events

Whenever possible, use the accepted Core or application correlation:

```text
LocalAccepted { operation_kind, correlation }
```

An input intent that was rejected by authority must not be logged as an accepted lifecycle event.
If no upstream correlation exists, the local stream assigns one ordinal at the sole append seam.

### Snapshot observations

Current snapshots are state, not an append-only event ledger. Snapshot-derived stream rows must be
typed as observations and coalesced by subject's immediately preceding fingerprint:

```text
ObservedStateTransition { subject, previous, current, observed_revision }
```

A global “seen text forever” set is forbidden. The sequence `A -> B -> A` represents three
observations and must retain the second `A` with a new stream ordinal.

Longer term, facts that are already represented by authoritative append-only events should stop
being synthesized from snapshots. That cleanup is useful but is not required before the delivery
transaction ships.

## Target Types

Names may change during implementation, but the ownership shape is required.

```rust
struct ParallelEventStreamSnapshot {
    generation: u64,
    first_ordinal: u64,
    events: Arc<[ProjectedParallelEvent]>,
}

struct ProjectedParallelEvent {
    id: ParallelStreamEventId,
    source: ParallelEventSourceId,
    line: Line<'static>,
}

struct ParallelHostBatch {
    expected_cursor: ParallelDeliveryCursor,
    commit_through: ParallelStreamEventId,
    events: Vec<ProjectedParallelEvent>,
}

struct ParallelLiveStreamModel {
    stream_generation: u64,
    after_cursor: ParallelDeliveryCursor,
    events: Vec<ProjectedParallelEvent>,
    title_visible: bool,
}

struct ParallelStreamDeliveryPlan {
    geometry: InlineGeometrySnapshot,
    host_batch: Option<ParallelHostBatch>,
    live_stream: ParallelLiveStreamModel,
}
```

Constructors and fields that could violate partitioning remain private. The plan constructor
validates stream generation, cursor range, retention gap, event ordering, and disjointness.

The event snapshot should use shared immutable storage so a terminal transaction can sample it
once without cloning every event merely to compare redraw state.

## Terminal Transaction

The terminal transaction follows two explicit commit phases.

```text
1. sample one semantic/frame input and event stream snapshot
2. capture terminal surface + geometry snapshot
3. prepare a pure parallel delivery plan
4. mark the exact host token Writing
5. execute the host insertion primitive
6. commit the host receipt or enter Uncertain
7. re-read geometry
8. replan the live model against the committed frontier when geometry changed
9. draw the owned frame
10. commit the independent frame receipt
```

The transaction may use bounded retries before any physical write. After a host receipt commits,
all retries begin from the new delivery frontier.

If post-insertion geometry requires additional durable events, the transaction either performs a
bounded second plan or defers the frame. It must not render an event at or below the committed
frontier.

## Terminal State Structure

The existing history flush state combines unrelated identities. The target separates them while
retaining one terminal coordinator:

```rust
struct TerminalDeliveryState {
    transcript: TranscriptDeliveryState,
    parallel: ParallelDeliveryState,
    geometry: HostScrollbackGeometryState,
    frame: FrameDeliveryState,
}
```

- transcript delivery may retain its existing semantic handoff token;
- parallel delivery uses event IDs and a frontier, never rendered-line overlap;
- geometry owns visible rows, reflow guards, and dirty recovery state; and
- frame delivery owns back-buffer trust and frame-attempt receipts.

The current parallel rendered-line baseline swap is removed. Shared terminal primitives may remain
shared, but conversation and parallel identity algorithms must be separate.

## Render Boundary

The production renderer receives:

- an owned `InlineShellFrameModel`;
- a `ParallelLiveStreamModel` for the active parallel stream; and
- no event window, host batch, delivery cursor, terminal state, or runtime capability.

The append-only stream renderer draws the supplied live events. It does not recalculate which
events belong to host scrollback. Geometry-specific row placement may remain pure renderer logic,
but delivery partitioning is already fixed by the terminal plan.

This narrows the consequence of an incorrect layout calculation. It may produce unused space or a
different number of undelivered live rows, but it cannot make a committed event live again.

## Reset And Mode Semantics

Every terminal reset must use an explicit policy:

```rust
enum TerminalSurfaceTransition {
    PreserveHostScrollback,
    ClearHostScrollback,
    ReplaceSurface,
}
```

- preserving host scrollback preserves the delivery frontier;
- clearing host scrollback may create a new surface generation and reset its frontier;
- replacing a surface rejects all older receipts; and
- switching between `HostScrollback` and `ViewportReplay` cannot implicitly reset or advance a
  host frontier.

`ViewportReplay` renders retained events without host writes. Returning to `HostScrollback` uses
the preserved host frontier and must not infer delivery from what was previously visible.

## Executable Architecture Guardrails

The architecture suite must protect invariants rather than the current helper spelling.

Required guards:

- `ParallelStreamEventId` is present on every projected stream event;
- one canonical event window exists; separate mutable live/scrollback deques are rejected;
- production renderer input uses `ParallelLiveStreamModel`, not
  `parallel_supervisor_event_lines: Vec<Line<'static>>`;
- renderer modules cannot import `ParallelHostBatch`, `ParallelDeliveryState`, or terminal write
  capabilities;
- parallel delivery does not call rendered-line `starts_with`, shifted overlap, or baseline
  `mem::swap`;
- delivery cursor fields are private and only the terminal delivery state machine can advance
  them;
- host receipt outcome routing is exhaustive;
- stream generation and expected frontier are checked before commit; and
- the retired line-based scrollback/live-tail architecture guard is removed.

AST guards cannot prove terminal behavior. They prevent the unsafe dependency and mutation shapes;
the temporal tests below prove behavior.

## Validation Contract

Implementation starts with a frame-recorder reproduction of the reported failure:

- one authority event;
- long wrapped GitHub identity error text;
- asynchronous parallel snapshot refresh;
- event transition across the host/live boundary; and
- exactly one occurrence in combined terminal history.

Required automated proof:

### Pure stream state

- runtime sequence stale and duplicate rejection;
- mixed-source total ordering;
- snapshot `A -> B -> A`;
- stream-generation ABA rejection;
- retention-gap production; and
- copy or language change does not alter event identity.

### Pure delivery state machine

- monotonic frontier;
- durable/live disjointness;
- duplicate receipt idempotence;
- stale expected frontier rejection;
- stale terminal surface rejection;
- committed host receipt survives frame failure;
- pre-write abort is retryable;
- ambiguous post-start failure enters `Uncertain`; and
- no operation sequence decreases the frontier.

Use a deterministic operation-sequence model test over append, resize, focus, overlay, draw
failure, host failure, retry, and mode switch. This gives property-like coverage without requiring
a new test dependency.

### Terminal integration

- long ASCII and Korean/CJK wrapped events;
- narrow and wide widths;
- shrink and restore;
- focused and passive parallel surfaces;
- focus loss and reacquisition;
- `StandardScrollRegion`;
- `NewlineFallback`;
- `HostScrollback`;
- explicit `ViewportReplay`;
- no panel chrome in host history; and
- every successful event ID appears exactly once across host plus live views.

### Primitive evidence

Because the implementation changes shared host-scrollback behavior, it follows the primitive-
sensitive capture contract. The PR records the required first-class environments and any explicit
downgrade. TestBackend or snapshot output alone is insufficient.

## Delivery Slices

Implementation should use two reviewable worktree/PR slices, each based on the latest
`origin/prerelease`.

### Slice 1: canonical event identity

- add this decision to the canonical architecture links;
- add typed stream generation, ordinal, and source identity;
- replace dual event stores with one event window;
- replace global rendered-text snapshot deduplication with source-specific transition handling;
- publish one immutable stream snapshot;
- retain current visible behavior;
- add stream stale, duplicate, ABA, and retention tests; and
- add guards preventing the old dual-store shape from returning.

This slice does not change terminal primitives.

### Slice 2: typed terminal delivery

- add the dedicated parallel delivery state and cursor;
- prepare a single host/live partition;
- split host and frame receipts;
- remove parallel rendered-line diff and baseline swapping;
- pass only `ParallelLiveStreamModel` to rendering;
- add explicit uncertain-write recovery;
- replace the retired line-based architecture guard;
- add the exact failure reproducer and transaction model tests;
- run the full native/TUI checks; and
- attach primitive-sensitive terminal evidence.

Each slice follows:

```text
worktree -> commit -> push -> PR(prerelease) -> rebase merge -> worktree cleanup
```

Slice 2 must not merge while both the old line baseline and the new delivery frontier can write
the same parallel stream.

## Completion Criteria

The implementation reaches the project 85% checkpoint only when all critical conditions below are
complete:

- stable event identity reaches terminal delivery;
- one canonical event window replaces dual storage;
- terminal delivery uses a monotonic typed frontier;
- durable and live models are disjoint;
- host and frame receipts commit independently;
- stale, duplicate, and ABA receipts cannot advance or reset delivery;
- renderer cannot access durable events or delivery state;
- ambiguous terminal writes do not automatically replay;
- exact long-line and resize reproducer passes;
- required architecture guards pass;
- native/TUI CI passes; and
- required terminal capture evidence is recorded.

The following may remain in the final 15%:

- converting every snapshot-derived event into a complete semantic payload enum;
- perfect file and module names;
- generalizing transcript and parallel delivery behind one generic engine;
- removing every compatibility adapter;
- exhaustive localization combinations; and
- aesthetic cleanup unrelated to delivery identity.

## Consequences

### Positive

- copy and layout changes cannot redefine event identity;
- a beginner cannot accidentally redraw committed events because renderer types do not contain
  them;
- resize and focus logic can invalidate frames without resetting host delivery;
- terminal failures become explicit state transitions;
- architecture tests block reintroduction of line-based parallel delivery;
- temporal tests focus on terminal behavior instead of compensating for missing identity; and
- the change preserves current hexagonal and Client Runtime boundaries.

### Costs

- the terminal adapter gains an explicit delivery state machine;
- frame capture and terminal sync must carry typed event IDs;
- primitive-sensitive implementation requires broad terminal evidence;
- snapshot-derived observations need clearer source correlation; and
- uncertain terminal writes require a visible recovery policy.

These costs replace existing implicit state and repeated reconciliation fixes; they do not add a
new product or business layer.

## Review Questions

Implementation review must answer:

1. Can any code path still identify a parallel event by rendered text?
2. Can an event be independently stored in both live and scrollback collections?
3. Can a renderer receive an event at or below the committed frontier?
4. Can a resize, focus change, overlay transition, or mode switch decrease the frontier?
5. Can a successful host insertion be replayed because the following frame failed?
6. Can an ambiguous write retry without entering an explicit recovery state?
7. Can a stale stream or terminal-surface receipt mutate current delivery?
8. Does the exact reported temporal sequence pass in both model and terminal tests?

If any answer violates the invariants above, the terminal delivery change is not reviewable.
