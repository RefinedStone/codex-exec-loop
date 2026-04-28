# Repo-Shared Planning Authority And Runtime Domain

Historical reference: the authority-store redesign described here is now implemented on the current
branch.

Use [../supersession/current-contract.md](../supersession/current-contract.md) for the current
operator-facing contract and
[../design/06-planning-runtime-and-draft-editor.md](../design/06-planning-runtime-and-draft-editor.md)
for current technical detail.

This document captures the forward design that replaced worktree-local planning authority on
`prerelease`. It should be read as implementation background, not as the active contract.

## Summary

The core redesign is no longer just "put planning into SQLite".

The closed architectural decision is:

- one repo-scoped authority domain owns planning state and supersession coordination state
- operator authoring still uses `draft -> validate -> promote`
- tracked planning files remain review and portability artifacts, not authority
- runtime recovery uses committed store state plus external truth recheck

Any implementation that keeps separate authorities for planning, queue claims, or runtime delivery
state is out of scope for this design.

## Closed Architectural Decisions

### 1. One Repo-Scoped Authority Domain

The canonical repo root owns one authority domain for:

- directions and direction detail content
- task ledger rows and queue projection
- queue-idle metadata
- store-backed drafts, promotion results, and rejection archives
- official refresh claims
- distributor queue claims
- supersession slot, session, and delivery projections
- runtime-domain events

This is one consistency domain even if the physical schema uses multiple tables.

The design explicitly rejects:

- planning authority in one store and queue claims in another
- a separate runtime-state authority that must coordinate with planning through best-effort updates

### 2. Store-Backed Draft And Promote Model

The current operator mental model stays intact:

- draft
- validate
- promote

The redesign does not permit direct mutation of active authority as the main authoring path.

Required consequences:

- drafts become store-backed state, not worktree-local authority files
- active planning remains immutable while a draft is being edited
- promote is the single transaction that can replace or merge active planning
- rejected promote attempts preserve resume-able draft context and rejection metadata

Structured change requests still exist, but they mutate draft state by default. They do not become
direct active-authority writes.

No filesystem mailbox is implied by this design. Structured change requests are authority-service
input shapes used by explicit CLI, TUI, or import flows, not files that the runtime watches and
auto-consumes from a directory.

### 3. Git-Tracked Files Become Exported Review Surfaces

The authority store lives under the canonical repo root:

| Path | Role |
| --- | --- |
| `.codex-exec-loop/runtime/planning-authority.db` | SQLite authority store |
| `.codex-exec-loop/runtime/planning-authority.lock` | repo-scoped advisory lock for migration, import, and recovery sweep |

Tracked planning files under `.codex-exec-loop/planning/` remain valuable, but only as:

- exported review artifacts
- explicit import sources
- branch-visible diffs when the operator chooses to mirror authority outward

They are not runtime authority anymore.

This intentionally changes the continuity rule:

- Git is no longer the primary carrier of authoritative planning state
- Git remains a review and portability surface through explicit export or import

### 4. Rollout Uses One-Way Mirror Stages Only

The compatibility path is:

| Mode | Meaning |
| --- | --- |
| `legacy-file` | tracked files remain authority; the store is not runtime authority |
| `shadow-store` | tracked files remain authority; the store mirrors and validates parity only |
| `store-primary` | the store is authority; tracked files are export and explicit import artifacts only |

`hybrid-read` is intentionally dropped because it reads like two-way sync.

The design explicitly rejects:

- long-lived dual authority
- automatic two-way synchronization between tracked files and the store

### 5. Planning Revision And Runtime Event Sequence Are Separate

Two independent monotonic counters are required.

`planning_revision` covers:

- planning catalog changes
- draft promotion into active state
- task ledger changes
- queue projection changes

`runtime_event_sequence` covers:

- refresh reservation and completion events
- queue claims
- push, PR, integration, and cleanup events
- redistribution or recovery markers

Each runtime event records the `observed_planning_revision` seen when the event was appended.

Planning mutation does not happen on every runtime event, and runtime churn must not inflate the
meaning of planning revision.

### 6. Recovery Requires External Truth Reconciliation

Recovery is not plain event replay.

The recovery contract is:

- use committed store projections and claims as the starting point
- re-check Git truth through `GitWorkspacePort`
- re-check GitHub truth through `GithubAutomationPort`
- re-check filesystem and worktree truth through the canonical repo root
- then reclassify in-flight work as recovered, blocked, failed, or already complete

The design explicitly rejects:

- declaring push, PR, or integration success from local projection state alone
- restart logic that relies only on event replay without external revalidation

## Boundary Model

### PlanningAuthorityLocator

Maps any active workspace path to:

- canonical repo root
- repo-shared runtime directory
- authority store path

This is the first place that stops treating a leased worktree as an independent planning root.

### PlanningAuthorityPort

This is the application-facing authority contract.

It owns:

- loading active planning snapshot
- loading and editing draft snapshot
- validating draft state
- promoting draft to active state
- recording hidden planner refresh results
- claiming official refresh work
- claiming distributor queue head ownership
- appending runtime-domain events
- reading and updating runtime projections
- exporting tracked planning artifacts
- importing tracked planning artifacts under explicit policy
- running recovery reconciliation

This port owns repo-scoped consistency, not raw file I/O.

### PlanningAuthorityBackend

Backend boundary below `PlanningAuthorityPort`.

v1 backend:

- SQLite

Required backend capabilities:

- schema migration
- read and write transactions
- compare-and-set style claim operations
- durable draft storage
- separate planning revision and runtime event sequence
- durable runtime projection and recovery markers

Possible future replacements:

- Rust-native append-log and snapshot store
- embedded custom storage engine

### Compatibility And Export Bridge

Compatibility logic remains an adapter concern.

It owns:

- one-time import from tracked planning files
- revision-stamped export views
- parity diagnostics in `shadow-store`
- explicit import validation in `store-primary`

## Authority Data Model

The physical schema may vary, but the logical model must include:

| Logical bucket | Required contents |
| --- | --- |
| `authority_metadata` | active mode, schema version, planning_revision, last export revision |
| `planning_active_*` | active catalog, directions, detail content, task rows, dependency edges, queue projection |
| `planning_drafts` | draft content, draft revision base, dirty state, validation state |
| `planning_rejections` | rejected promote attempts, restore metadata, resume linkage |
| `runtime_claims` | refresh claims, queue-head claims, recovery ownership, expiry metadata |
| `runtime_projections` | slot, session, queue, and distributor delivery projections |
| `runtime_events` | append-only orchestration events with `runtime_event_sequence` and `observed_planning_revision` |

The authority schema must store:

- queue-idle prompt content
- result-output guidance
- direction detail markdown
- active snapshot metadata
- export metadata

Tracked file paths are export-shape concerns, not the primary authority schema.

## Transaction Rules

The minimal active commit unit is:

- validated active or promoted planning mutation
- rebuilt queue projection
- updated planning_revision
- any claim or event writes that must be atomic with that mutation

Examples that must be single-transaction:

- draft promotion into active state
- hidden planner refresh that changes tasks and queue head
- official completion that marks a task done and emits follow-up tasks
- repair rollback that restores accepted state and projection metadata
- queue-head claim that depends on a specific planning_revision

## Delivery Model

### Draft Mutation Flow

1. operator edits draft state
2. draft validation runs against store-backed draft content
3. promote attempts one transactional active update
4. success replaces or merges active snapshot and rebuilds queue projection
5. failure writes rejection metadata without corrupting active state

### Runtime Coordination Flow

1. agent reports completion
2. authority store records non-official runtime event
3. one process claims official refresh responsibility
4. hidden planning worker refreshes active planning state
5. a new planning_revision commits if refresh succeeds
6. distributor claims one queue head against a committed revision
7. delivery events and projections advance
8. recovery may later re-check external truth and reconcile state

## Remaining Implementation Slices

### Slice 9: Authority Locator And Shadow Store

- add canonical repo authority resolution
- add SQLite schema and read-only snapshot loading
- mirror tracked planning files into the store without changing runtime authority

### Slice 10: Store-Backed Drafts And Promote

- move draft storage and validation into the authority store
- keep draft, validate, and promote UX intact
- prove that active authority stays unchanged until promote succeeds

### Slice 11: Active Planning Mutation And Queue Claims

- route hidden planner refresh and official completion claims through the authority store
- migrate queue-head claim semantics into the same authority domain

### Slice 12: Runtime Projection Migration And Recovery

- move slot, session, queue, and distributor projections into the store
- append runtime-domain events
- add recovery reconciliation against Git and GitHub truth

### Slice 13: Store-Primary Cutover

- make store-backed active planning the default runtime authority
- keep tracked planning files as revision-stamped exports
- allow explicit import only under controlled operator flow

## Acceptance Criteria

- every worktree under one canonical repo root sees the same active planning revision
- drafts are shared by repo scope and do not mutate active state until promote succeeds
- hidden planner refresh from a leased worktree mutates repo-scoped active authority, not local
  planning files
- official refresh claims and distributor queue-head claims live in the same authority domain as
  planning revision
- planning revision and runtime event sequence remain distinct
- recovery can reclassify in-flight push, PR, integration, and cleanup work by rechecking external
  truth
- tracked planning files remain readable and reviewable without becoming runtime authority again

## Related Docs

- [19-supersession-runtime-risk-audit.md](19-supersession-runtime-risk-audit.md)
- [17-structure-and-architecture-debt-map.md](17-structure-and-architecture-debt-map.md)
- [16-planning-and-automation-evolution.md](16-planning-and-automation-evolution.md)
- [../design/06-planning-runtime-and-draft-editor.md](../design/06-planning-runtime-and-draft-editor.md)
- [../supersession/current-contract.md](../supersession/current-contract.md)
