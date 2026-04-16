# Structure And Architecture Debt Map

This document maps structural debt to operator-visible product costs so future refactors stay tied to UX and reliability outcomes.

## Guiding Principle

Refactors are justified when they make the shell easier to reason about, easier to evolve safely, or easier to recover when something goes wrong.

This is not a style-cleanup list. It is a product-facing debt map.

## Debt Map

| Area | Current hotspot | Operator-facing cost | Target boundary |
| --- | --- | --- | --- |
| shell presentation | `src/adapter/inbound/tui/app/shell_presentation.rs`, `src/adapter/inbound/tui/app/shell_rendering.rs`, `src/adapter/inbound/tui/app/shell_presentation/status_panels.rs` | status meaning is spread across rendering and presentation code, making UX iteration harder | separate state wording, layout policy, and overlay projection |
| conversation runtime | `src/adapter/inbound/tui/app/conversation_runtime.rs`, `conversation_model.rs`, `conversation_model/view_model.rs` | turn lifecycle, auto-follow state, and shell status compete in one runtime surface | separate conversation lifecycle from automation lifecycle and surface projection |
| planning authoring flow | `src/adapter/inbound/tui/app/planning/controller.rs`, `planning_draft_editor_ui.rs` | planning setup, editor safety, and directions maintenance feel coupled | separate planning setup flow, authoring flow, and close-risk handling |
| planning runtime services | `src/application/service/planning_directions_service.rs`, `src/application/service/planning_validation_service.rs`, `src/application/service/planning_reconciliation_service.rs`, `src/application/service/planning_prompt_service.rs` | planning concepts are powerful but spread across many services with overlapping product language | distinguish authoring, validation, runtime projection, and recovery more sharply |
| automation policy and queue behavior | `src/application/service/planning_runtime_policy_service.rs`, `src/application/service/planning_runtime_facade_service.rs`, `src/application/service/planning_worker_orchestration_service.rs` | queue-driven continuation is hard to explain because policy, prompting, and recovery are separated differently than the operator sees them | align automation policy surface with operator concepts such as next task, pause reason, and resume path |
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
- directions and queue-idle supporting files

Should not own:

- post-turn runtime evaluation
- generic queue explanation

### 4. Planning Runtime Boundary

Owns:

- accepted planning snapshot
- queue derivation
- proposal semantics
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

### Stream B: Presentation And Status Extraction

Split shell wording and projection concerns from layout and rendering concerns. This makes UX work cheaper without changing planning internals first.

### Stream C: Planning Authoring Versus Planning Runtime

Separate authoring flows from runtime queue and automation flows so planning can become easier to explain and easier to evolve independently.

### Stream D: Runtime Safety And Test Shape

Reorganize runtime tests around operator journeys and architectural contracts rather than only around current file boundaries.

## Definition Of Done For Each Refactor Slice

- one clear ownership boundary becomes easier to explain in docs
- one operator-visible flow becomes easier to reason about
- one hotspot file or cluster loses mixed responsibilities
- matching tests describe user-visible intent more clearly than before

## Explicit Assumptions

- The inline shell remains the only frontend in the near term.
- File-backed planning remains the trusted source of work continuity.
- Architectural work is only worth taking when it improves UX clarity, planning trust, or runtime safety.
