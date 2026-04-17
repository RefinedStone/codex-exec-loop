# Repo-Shared Planning Authority Store

This document defines the planned replacement for worktree-local planning authority on `prerelease`.
It is a forward design and development document, not shipped behavior.

## Problem Statement

The current supersession runtime already treats slot leases, agent session detail, and distributor
queue state as repo-shared concerns, but planning authority still lives under the active workspace
path.

That mismatch creates three structural failures:

- planning authority can split across the root checkout and leased worktrees
- `task-ledger.json` and `queue.snapshot.json` are updated through separate file operations
- process-local ordering and file-backed queue writes do not become cross-process safety just
  because they run in one repository family

Improving the priority queue alone does not solve this. The queue is a derived projection. The
missing piece is a single repo-scoped authority store with transactional mutation rules.

## Goals

- make planning authority repo-shared across every worktree that belongs to one canonical git root
- move all planning authority into one store: directions, detail content, queue policy, tasks, and
  queue projection
- make ledger and queue projection updates transactional
- keep a backend boundary so SQLite can be replaced later by a Rust-native store
- keep operator-visible editing and review flows through structured import/export surfaces
- add runtime-domain events for supersession orchestration without turning all planning authoring
  into full event sourcing

## Non-Goals

- full event sourcing for directions authoring and task editing
- using git refs or git objects as the primary authority model in v1
- supporting long-term dual authority where files and the store are both official
- preserving worktree-local planning authority semantics under supersession

## Repo-Shared Runtime Area

Authority state should live under the canonical repo root, not under an individual worktree.

Recommended layout:

| Path | Role |
| --- | --- |
| `.codex-exec-loop/runtime/planning-authority.db` | SQLite authority store |
| `.codex-exec-loop/runtime/planning-authority.lock` | repo-scoped advisory lock for migration, recovery, and cutover operations |
| `.codex-exec-loop/runtime/exports/planning-snapshot.json` | full exported review snapshot, not authority |
| `.codex-exec-loop/runtime/exports/task-ledger.json` | exported ledger view, not authority |
| `.codex-exec-loop/runtime/exports/queue.snapshot.json` | exported queue view, not authority |
| `.codex-exec-loop/runtime/imports/planning-change-request.json` | optional structured import request for operator-facing editing flows |

The legacy tracked files under `.codex-exec-loop/planning/` become compatibility artifacts only.
They are imported from or exported to the authority store depending on the selected compatibility
mode, but they are no longer the runtime source of truth.

## Boundary Model

### PlanningAuthorityLocator

Maps any active workspace path to:

- canonical repo root
- repo-shared runtime directory
- authority store path

This is the first place that must stop treating a leased worktree as a separate planning authority.

### PlanningAuthorityService

The only application-facing authority API.

It owns:

- loading committed planning snapshots
- applying validated planning mutations
- refreshing queue projections
- recording hidden planner refresh results
- recording official completion outcomes
- importing legacy planning files
- exporting review/debug views

It does not expose raw file-write operations as the main contract.

### PlanningAuthorityBackend

Backend boundary below `PlanningAuthorityService`.

v1 backend:

- SQLite

Possible future replacements:

- Rust-native append-log and snapshot store
- embedded custom storage engine

Required backend capabilities:

- schema migration
- read/write transaction support
- compare-and-set style claim/update operations
- revision tracking
- durable runtime event append

### PlanningCompatibilityBridge

Translates between legacy planning files and the authority store.

It owns:

- one-time import of legacy directions and task ledger files
- compatibility export views
- mode-based routing between legacy and store-backed operation
- mismatch diagnostics during rollout

### SupersessionRuntimeState Store

The same repo-shared authority area should also hold supersession runtime state that must survive
restart and coordinate across worktrees.

This state is not the same thing as planning authoring state, but it belongs beside it because both
need repo-scoped consistency.

## Why This Still Fits Hexagonal Architecture

This redesign still fits the existing architectural direction, but the boundary must move up from
"file read/write" to "authority transaction and projection management".

- application services should depend on authority operations such as snapshot load, mutation apply,
  queue claim, and export
- SQLite remains an outbound adapter behind the backend boundary, not a new application center
- compatibility import and export stay in adapters
- the priority queue remains a domain projection, not the persistence model itself

The key change is not abandoning ports and adapters. The key change is introducing a port with the
right level of responsibility for repo-shared authority and concurrency.

## Authority Model

The authority store becomes the official source of truth for:

- directions catalog
- direction detail content
- queue-idle policy and review prompt content
- task ledger rows and dependency edges
- queue projection
- plan-enabled and revision metadata
- repair and rejection metadata
- supersession runtime events
- supersession slot/session/distributor projections

Files become:

- import source
- export view
- operator review surface
- debug artifact

This means `task-ledger.json` and `queue.snapshot.json` may still exist, but only as generated or
imported views. They must not be treated as concurrent writable authority files anymore.

## Data Model

### Planning Tables

| Table | Purpose |
| --- | --- |
| `authority_metadata` | backend mode, current revision, migration metadata, last export revision |
| `planning_catalog_state` | repo-scoped planning settings such as queue-idle policy, queue-idle prompt markdown, result-output markdown, and plan-enabled state |
| `planning_directions` | direction rows including title, summary, state, success criteria, scope hints, and detail markdown |
| `planning_tasks` | task rows with normalized queue fields and full payload metadata |
| `planning_task_edges` | dependency and blocker relations |
| `planning_queue_projection` | committed projection rows for next task, active tasks, proposed tasks, and skipped tasks |
| `planning_rejections` | archived invalid candidate metadata and repair linkage |

### Supersession Runtime Tables

| Table | Purpose |
| --- | --- |
| `supersession_runtime_events` | append-only runtime-domain event log |
| `supersession_runtime_state` | latest slot, session, queue-head, and delivery projection |
| `supersession_claims` | queue-head claims, refresh claims, and recovery ownership markers |

### Revision Rule

Every committed planning mutation bumps a single repo-scoped revision number.

That revision covers:

- planning catalog changes
- directions changes
- task ledger changes
- queue projection changes

Readers only consume committed revisions. Export views must record which revision they represent.

## Editing Contract

Operator-facing editing should move from “edit active authority files directly” to “submit a
structured change request”.

Recommended JSON request envelope:

```json
{
  "base_revision": 42,
  "changes": [
    {
      "op": "upsert_direction",
      "direction": {
        "id": "supersession-architecture-boundaries",
        "title": "Supersession architecture boundaries",
        "summary": "Split supersession into explicit repo-shared boundaries.",
        "state": "active",
        "success_criteria": [
          "Parallel mode uses a repo-shared planning authority store."
        ],
        "scope_hints": [
          "Prefer transactional authority updates over direct file mutation."
        ],
        "detail_markdown": "# Detail\n..."
      }
    },
    {
      "op": "upsert_task",
      "task": {
        "id": "task-planning-authority-store-bootstrap",
        "direction_id": "supersession-architecture-boundaries",
        "title": "Bootstrap repo-shared planning authority store",
        "status": "ready",
        "base_priority": 96,
        "updated_at": "2026-04-18T12:00:00Z"
      }
    },
    {
      "op": "set_queue_idle",
      "policy": "review_and_enqueue",
      "prompt_markdown": "# queue-idle review\n..."
    }
  ]
}
```

Design rules:

- the request is optimistic and revision-based
- the request is the mutable user-facing shape, not the internal canonical schema
- the authority service validates and normalizes the request before commit
- legacy file imports are translated into the same internal mutation model

## Runtime Event Model

Runtime-domain events are introduced only for supersession orchestration.

Recommended v1 event types:

- `agent_completion_reported`
- `official_refresh_reserved`
- `official_refresh_started`
- `official_refresh_succeeded`
- `official_refresh_failed`
- `commit_ready_enqueued`
- `distributor_queue_head_claimed`
- `source_branch_push_started`
- `source_branch_push_failed`
- `pull_request_ensure_started`
- `pull_request_ensure_failed`
- `integration_started`
- `integration_succeeded`
- `integration_failed`
- `slot_cleanup_started`
- `slot_cleanup_succeeded`
- `slot_cleanup_failed`
- `redistribution_requested`

Each event row should contain:

- monotonically increasing event sequence
- repo-scoped authority revision
- event type
- aggregate key such as task id, slot id, session key, or queue item id
- JSON payload
- timestamp

The runtime state tables are projections of these events plus claim tables, not ad hoc files under
the pool root.

## Transaction Model

The minimal committed transaction unit is:

- accepted planning mutation
- validated task state
- rebuilt queue projection
- new committed revision

Examples that must be single-transaction:

- hidden planner refresh that changes tasks and next queue head
- proposal promotion into executable queue
- official completion that marks a task done and emits follow-up tasks
- repair rollback that restores accepted state and projection metadata

`queue.snapshot.json` should be produced from committed store state after the transaction, not
written as a second authority step.

## Concurrency Model

### Short Transactions

Use SQLite write transactions for normal planning mutations.

The expected pattern is:

- `BEGIN IMMEDIATE`
- validate base revision
- apply mutation
- rebuild projection
- append any related runtime event
- bump revision
- `COMMIT`

### Claims

Use row-level claim semantics through guarded updates in the store for:

- official completion refresh reservation
- distributor queue-head claim
- recovery worker ownership

The claim model must be compare-and-set style, not “scan files and hope nobody else writes first”.

### Advisory Lock

Use the repo-scoped lock file only for long-lived global operations such as:

- first-time migration
- legacy import/cutover
- recovery sweep

Do not hold an OS lock across the full lifetime of distributor delivery.

## Compatibility Modes

The new authority layer should support three explicit modes.

| Mode | Meaning |
| --- | --- |
| `legacy-file` | current file-backed planning remains the runtime authority |
| `hybrid-read` | legacy files can still be imported, but runtime reads and writes go through the authority store |
| `store-primary` | authority store is official and files are import/export views only |

Rollout order:

1. `legacy-file`
2. `hybrid-read`
3. `store-primary`

Long-term dual authority is explicitly out of scope.

## Implementation Slices

### Slice 1: Authority Locator And Read Path

- add canonical repo authority location rules
- bootstrap SQLite schema and read-only snapshot loading
- keep runtime behavior in `legacy-file` mode

### Slice 2: Planning Snapshot Shadow Mode

- build queue projection from the store
- export read-only snapshots for comparison against legacy files
- report divergence without cutting over writes yet

### Slice 3: Hidden Planner Write Path

- route hidden queue refresh and repair through the authority store
- make task and queue projection commit transactional
- keep compatibility export enabled

### Slice 4: Official Completion And Queue Claims

- move official completion refresh reservation and queue-head claims into the store
- replace process-local ordering guarantees with repo-shared claim semantics

### Slice 5: Supersession Runtime State Migration

- move slot/session/distributor state from pool-root files into store-backed projections
- append runtime-domain events for delivery lifecycle
- add recovery sweep on restart

### Slice 6: Store-Primary Cutover

- make store-backed planning the default runtime authority
- reduce legacy files to import/export and review artifacts
- remove assumptions that the active worktree owns planning authority

## Acceptance Criteria

- every worktree under one canonical repo root sees the same committed planning authority
- hidden planner refresh from a leased worktree mutates the repo-shared authority, not that
  worktree's local planning files
- `task ledger` and queue projection become visible only at the same committed revision
- two processes cannot both claim the same official completion refresh order
- two processes cannot both claim the same distributor queue head
- restart recovery can reconstruct in-flight supersession delivery state from committed store data
- operator-facing planning exports remain readable without becoming authority again

## Related Docs

- [19-supersession-runtime-risk-audit.md](19-supersession-runtime-risk-audit.md)
- [17-structure-and-architecture-debt-map.md](17-structure-and-architecture-debt-map.md)
- [../design/06-planning-runtime-and-draft-editor.md](../design/06-planning-runtime-and-draft-editor.md)
- [../supersession/09-architecture-boundaries.md](../supersession/09-architecture-boundaries.md)
