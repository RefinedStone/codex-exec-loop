# Structure And Architecture Debt Map

This document maps structural debt to operator-visible product costs so future refactors stay tied to UX and reliability outcomes.

## Guiding Principle

Refactors are justified when they make the shell easier to reason about, easier to evolve safely, or easier to recover when something goes wrong.

This is not a style-cleanup list. It is a product-facing debt map.

Small-context readability is part of that product cost. If a flow requires opening a giant service file
plus unrelated infrastructure adapters before the behavior becomes legible, the structure is already
hurting both implementation safety and review quality.

## Current Hotspot Order

Use this order for the current cycle when choosing a refactor slice:

1. shell presentation: `src/adapter/inbound/tui/app/shell_presentation.rs` and nearby rendering or projection files
2. conversation runtime: `src/adapter/inbound/tui/app/conversation_runtime.rs`
3. planning controller: `src/adapter/inbound/tui/app/planning/controller.rs` and nearby authoring UI
4. parallel mode service: `src/application/service/parallel_mode_service.rs`

If a change starts with a later hotspot, record why an earlier hotspot was skipped.

## Debt Map

| Area | Current hotspot | Operator-facing cost | Target boundary |
| --- | --- | --- | --- |
| shell presentation | `src/adapter/inbound/tui/app/shell_presentation.rs`, `src/adapter/inbound/tui/app/shell_rendering.rs`, `src/adapter/inbound/tui/app/shell_presentation/status_panels.rs` | status meaning is spread across rendering and presentation code, making UX iteration harder | separate state wording, layout policy, and overlay projection |
| conversation runtime | `src/adapter/inbound/tui/app/conversation_runtime.rs`, `conversation_model.rs`, `conversation_model/view_model.rs` | turn lifecycle, continuation state, and shell status compete in one runtime surface | separate conversation lifecycle from continuation lifecycle and surface projection |
| planning controller | `src/adapter/inbound/tui/app/planning/controller.rs`, `planning_draft_editor_ui.rs` | planning setup, editor safety, and direction-side authoring feel coupled | separate planning setup flow, authoring flow, and close-risk handling |
| parallel mode service | `src/application/service/parallel_mode_service.rs` | storage, recovery, queue, slot, and snapshot concerns compete in one hotspot, so safe edits require too much context | split into readiness, slots, distributor, recovery, snapshot, and completion boundaries |
| planning runtime services | `src/application/service/planning/authoring/directions.rs`, `src/application/service/planning/runtime/validation.rs`, `src/application/service/planning/repair/reconciliation.rs`, `src/application/service/planning/runtime/prompt.rs` | planning concepts are powerful but spread across many services with overlapping product language | distinguish authoring, validation, runtime projection, and recovery more sharply |
| continuation policy and queue behavior | `src/application/service/planning/runtime/policy.rs`, `src/application/service/planning/runtime/facade.rs`, `src/application/service/planning/worker/orchestration.rs` | queue-driven continuation is hard to explain because policy, prompting, and recovery are separated differently than the operator sees them | align continuation policy surface with operator concepts such as next task, pause reason, and resume path |
| outbound infrastructure layout | `src/adapter/outbound/` as one flat directory | DB, GitHub, filesystem, and app-server details are harder to skip when tracing feature logic | group outbound adapters by infrastructure boundary and keep composition near entrypoints |
| broad integration-style tests | `src/adapter/inbound/tui/app/app_tests.rs` and planning runtime test clusters | behavior is well-covered, but intent is harder to discover for future changes | split tests by operator journey and subsystem contract |

## Planning Hotspot Audit

Composition wiring is no longer the planning hotspot. The planning feature composition now delegates
shared services plus workspace, runtime, and worker dependency bundles before constructing public
use-case groups.

The remaining planning hotspots by current implementation size and mixed responsibility are:

| Rank | Hotspot | Current pressure | Narrow next slice |
| --- | --- | --- | --- |
| 1 | `src/application/service/planning/admin.rs` and `src/application/service/planning/admin/*` | admin DTOs still share the facade file with `PlanningAdminFacadeService` while overview/runtime, file sync, draft wrapper, and reset orchestration move behind child modules | keep shrinking admin child-module boundaries while preserving `PlanningAdminFacadeService` and exported DTOs |
| 2 | `src/application/service/planning/repair/reconciliation.rs` | repair orchestration, protected-file restore, prompt construction, focused ledger excerpts, and tests share one module | split repair prompt construction and focused excerpt helpers behind the repair boundary |
| 3 | `src/adapter/inbound/tui/app/planning/controller.rs` | shell command dispatch, setup flow, draft editor close-risk handling, reset parsing, and status copy share one controller impl | split reset/status-copy helpers before moving effectful controller paths |
| 4 | `src/application/service/planning/authoring/directions.rs` | direction summary, supporting-file staging, doctor repair, and path rewriting share one authoring service | split supporting-file path rewrite helpers from the service methods |
| 5 | `src/application/service/planning/runtime/validation.rs` and `src/application/service/planning/runtime/prompt.rs` | validation rules and runtime projection assembly are large but already stay inside runtime boundaries | extract rule groups only after admin and repair boundaries are clearer |

Queued next narrow slice:

- **Task:** Split planning admin file sync orchestration from `PlanningAdminFacadeService`.
- **Why next:** this planning-specific audit follows the already-started planning cleanup lane while
  the current top TUI hotspots remain untouched by this document-only queue update. After the
  projection, document mutation, draft session, and CRUD splits,
  `admin.rs` still owns effectful export/apply flows plus parallel-work blocking helpers. Moving
  that file sync orchestration next reduces the remaining facade context without changing behavior,
  ports, or operator flows.
- **Target write set:** `src/application/service/planning/admin.rs` and a new
  `src/application/service/planning/admin/file_sync.rs`.
- **Acceptance:** admin public exports remain unchanged; `export_active_files_for_edit` and
  `apply_exported_files` move behind the new module; parallel-work blocking and candidate-file write
  helpers move with the file sync flow; existing planning/admin tests continue to pass.

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
