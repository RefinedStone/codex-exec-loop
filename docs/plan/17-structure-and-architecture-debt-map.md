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
| conversation runtime | `src/adapter/inbound/tui/app/conversation_runtime.rs`, `conversation_model.rs`, `conversation_model/view_model.rs` | turn lifecycle, auto-follow state, and shell status compete in one runtime surface | separate conversation lifecycle from automation lifecycle and surface projection |
| planning controller | `src/adapter/inbound/tui/app/planning/controller.rs`, `planning_draft_editor_ui.rs` | planning setup, editor safety, and direction-side authoring feel coupled | separate planning setup flow, authoring flow, and close-risk handling |
| parallel mode service | `src/application/service/parallel_mode_service.rs` | storage, recovery, queue, slot, and snapshot concerns compete in one hotspot, so safe edits require too much context | split into readiness, slots, distributor, recovery, snapshot, and completion boundaries |
| planning runtime services | `src/application/service/planning_directions_service.rs`, `src/application/service/planning_validation_service.rs`, `src/application/service/planning_reconciliation_service.rs`, `src/application/service/planning_prompt_service.rs` | planning concepts are powerful but spread across many services with overlapping product language | distinguish authoring, validation, runtime projection, and recovery more sharply |
| automation policy and queue behavior | `src/application/service/planning_runtime_policy_service.rs`, `src/application/service/planning_runtime_facade_service.rs`, `src/application/service/planning_worker_orchestration_service.rs` | queue-driven continuation is hard to explain because policy, prompting, and recovery are separated differently than the operator sees them | align automation policy surface with operator concepts such as next task, pause reason, and resume path |
| outbound infrastructure layout | `src/adapter/outbound/` as one flat directory | DB, GitHub, filesystem, and app-server details are harder to skip when tracing feature logic | group outbound adapters by infrastructure boundary and keep composition near entrypoints |
| broad integration-style tests | `src/adapter/inbound/tui/app/app_tests.rs` and planning runtime test clusters | behavior is well-covered, but intent is harder to discover for future changes | split tests by operator journey and subsystem contract |

## Chosen Boundary Model

### 1. Shell Runtime Boundary

Owns:

- startup readiness
- active turn lifecycle
- session selection and shell mode transitions

Should not own:

- planning authoring semantics
- detailed automation copy decisions

### 2. Automation Boundary

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
