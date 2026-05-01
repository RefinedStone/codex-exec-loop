# Structure And Architecture Debt Map

This document maps structural debt to operator-visible product costs so future refactors stay tied
to UX and reliability outcomes.

## Guiding Principle

Refactors are justified when they make the shell easier to reason about, easier to evolve safely, or
easier to recover when something goes wrong.

This is not a style-cleanup list. It is a product-facing debt map.

Small-context readability is part of that product cost. If a flow requires opening a giant service file
plus unrelated infrastructure adapters before the behavior becomes legible, the structure is already
hurting both implementation safety and review quality.

## Current Hotspot Order

Use this order for the current cycle when choosing a refactor slice. Completed checkpoints stay here
only when they prevent repeated work.

1. parallel mode service child modules: `src/application/service/parallel_mode/pool.rs`,
   `src/application/service/parallel_mode/distributor.rs`, and supervisor wiring
2. planning runtime rule groups: `src/application/service/planning/runtime/validation.rs` and
   `src/application/service/planning/runtime/prompt.rs`
3. test shape: broad shell rendering and parallel-mode integration-style tests

If a change starts with a later hotspot, record why an earlier hotspot was skipped.

## Debt Map

| Area | Current hotspot | Operator-facing cost | Target boundary |
| --- | --- | --- | --- |
| shell presentation | `src/adapter/inbound/tui/app/shell_presentation/*` | the broad surface is now split, but overlay copy and layout still require several neighboring files | keep wording, projection, and layout modules separate |
| conversation runtime | `src/adapter/inbound/tui/app/conversation_runtime.rs`, `conversation_model.rs`, `conversation_model/view_model.rs` | current file sizes are controlled, but continuation and conversation status still need clear ownership when behavior changes | separate conversation lifecycle from continuation lifecycle and surface projection |
| planning controller | `src/adapter/inbound/tui/app/planning/controller.rs`, `controller/*`, `planning_draft_editor_ui.rs` | setup keys, status copy, and close-risk helpers are split; future work should avoid re-coupling effectful controller paths | keep setup, authoring, and close-risk handling in dedicated modules |
| parallel mode service | `src/application/service/parallel_mode/pool.rs`, `src/application/service/parallel_mode/distributor.rs`, `src/application/service/parallel_mode/session_detail.rs` | pool recovery, distributor delivery, and session history are split from the facade, but the larger child modules still require careful tracing | keep service modules focused on orchestration and keep pure readiness, roster, detail, slot, and cleanup decisions in `src/domain/parallel_mode` |
| planning runtime services | `src/application/service/planning/authoring/directions.rs`, `src/application/service/planning/runtime/validation.rs`, `src/application/service/planning/repair/reconciliation.rs`, `src/application/service/planning/runtime/prompt.rs` | planning concepts are powerful but spread across services with overlapping product language | keep semantic validation and queue facts in `src/domain/planning`; distinguish authoring, validation, runtime prompt assembly, and recovery more sharply |
| continuation policy and queue behavior | `src/application/service/planning/runtime/policy.rs`, `src/application/service/planning/runtime/facade.rs`, `src/application/service/planning/worker/orchestration.rs` | queue-driven continuation is hard to explain because policy, prompting, and recovery are separated differently than the operator sees them | align continuation policy surface with operator concepts such as next task, pause reason, and resume path |
| outbound infrastructure layout | `src/adapter/outbound/` as one flat directory | DB, GitHub, filesystem, and app-server details are harder to skip when tracing feature logic | group outbound adapters by infrastructure boundary and keep composition near entrypoints |
| broad integration-style tests | `src/adapter/inbound/tui/app/shell_rendering*_tests.rs`, `src/application/service/parallel_mode/tests/*`, and planning runtime test clusters | behavior is well-covered, but intent is harder to discover for future changes | split tests by operator journey and subsystem contract |

## Domain Extraction Status

Recent extraction work moved several formerly service-local calculations into domain types:

- `src/domain/parallel_mode.rs` now derives parallel readiness, supervisor state, pool slot state
  from lease state, cleanup-ready decisions, roster entries, selected detail, and live-detail
  enrichment.
- `src/domain/planning` now owns planning semantic validation, priority queue ordering and
  skip/proposal classification, queue visibility, and queue/proposal summary projection.
- The application layer should therefore focus new parallel-mode and planning work on orchestration,
  persistence, prompt assembly, and recovery side effects.

## Completed Boundary Checkpoints

- All non-reference source files under `src/` are below the 1000-line threshold.
- `src/adapter/inbound/tui/app/planning/controller.rs` is split into overlay key handlers and
  status/reset copy helpers under `planning/controller/`.
- `src/adapter/outbound/filesystem/planning_workspace.rs` no longer imports the concrete SQLite
  authority adapter; repo-scoped workspace behavior is injected through
  `RepoScopedPlanningWorkspacePort`.
- `src/adapter/outbound/db/sqlite_planning_authority_adapter.rs` keeps active-document,
  runtime-projection, repo-scoped-workspace, store, and path helpers in child modules.
- `src/application/service/planning/repair/reconciliation.rs` keeps guard tests and fixtures in
  `repair/reconciliation/tests.rs`.
- `src/application/service/planning/authoring/directions.rs` keeps supporting-file path validation,
  default detail-doc generation, queue-idle prompt normalization, and catalog path mutation helpers
  in `authoring/directions/supporting_files.rs`.

## Planning Hotspot Audit

Composition wiring is no longer the planning hotspot. The planning feature composition now delegates
shared services plus workspace, runtime, and worker dependency bundles before constructing public
use-case groups.

The remaining planning hotspots by current implementation size and mixed responsibility are:

| Rank | Hotspot | Current pressure | Narrow next slice |
| --- | --- | --- | --- |
| 1 | `src/application/service/planning/runtime/validation.rs` and `src/application/service/planning/runtime/prompt.rs` | validation rules and runtime prompt assembly are large but already stay inside runtime boundaries; queue facts have moved to domain | extract rule groups only when a behavior change touches them |
| 2 | `src/application/service/planning/admin/*` | admin facade and file-sync orchestration are now split into child modules, but exported DTOs still make the folder a broad public surface | keep admin submodules stable and move only clearly isolated admin projections or document helpers |

Queued next narrow slice:

- **Task:** Keep parallel-mode child modules below the line threshold while separating orchestration
  from pure projection and queue-delivery policy.
- **Why next:** planning repair and directions supporting-file slices are now split, while
  `parallel_mode/distributor.rs` and `parallel_mode/pool.rs` remain the largest service modules.
- **Target write set:** one `src/application/service/parallel_mode/*` child module at a time.
- **Acceptance:** existing parallel-mode distributor or pool focused tests continue to pass.

## Chosen Boundary Model

### 1. Shell Runtime Boundary

Owns:

- startup readiness
- active turn lifecycle
- session selection and shell mode transitions

Should not own:

- planning authoring semantics
- detailed continuation copy decisions

### 2. Continuation Boundary

Owns:

- stop rules
- queue-driven continuation decision
- pause reason projection
- preview assembly contract

Should not own:

- planning authoring flow
- generic shell layout

### 3. Planning Authoring Boundary

Owns:

- staging
- editing
- promoting
- close-risk and dirty-buffer semantics
- direction and queue-idle supporting files

Should not own:

- post-turn runtime evaluation
- generic queue explanation

### 4. Planning Runtime Boundary

Owns:

- accepted planning snapshot
- queue derivation
- proposed-task semantics
- runtime readiness states

Should not own:

- shell copy scattered across presentation layers

### 5. Repair And Worker Boundary

Owns:

- reconciliation
- rollback
- bounded hidden worker retries
- rejected artifact preservation

Should expose:

- why a change was rejected
- what was restored
- what action the operator should take next

## Sequencing Recommendation

### Stream A: Product Language First

Before deep refactors, normalize operator vocabulary in docs and presentation surfaces so later code moves have a stable target.
Use shared terms such as direction, queue task, proposed task, accepted planning, queue-idle policy, and repair before moving files around.

### Stream B: Presentation And Status Extraction

Split shell wording and projection concerns from layout and rendering concerns. This makes UX work cheaper without changing planning internals first.

### Stream C: Infrastructure Directory Separation

Move outbound adapters into clear technology boundaries so feature-level analysis can skip infra details unless a task really crosses that boundary.

### Stream D: Parallel Mode And Planning Service Split

Separate parallel-mode responsibilities and planning authoring/runtime/repair concerns so both flows become smaller and more legible to change.

### Stream E: Runtime Safety And Test Shape

Reorganize runtime tests around operator journeys and architectural contracts rather than only around current file boundaries.

## Definition Of Done For Each Refactor Slice

- one clear ownership boundary becomes easier to explain in docs
- one operator-visible flow becomes easier to reason about
- one hotspot file or cluster loses mixed responsibilities
- one infrastructure directory becomes easier to skip when it is not relevant to the current feature
- matching tests describe user-visible intent more clearly than before

## Explicit Assumptions

- The inline shell remains the only frontend in the near term.
- Planning remains operator-owned, but the long-term trusted source of continuity may move to a
  repo-shared authority store as long as staged drafts and exported review surfaces remain intact.
- Architectural work is only worth taking when it improves UX clarity, planning trust, or runtime safety.
