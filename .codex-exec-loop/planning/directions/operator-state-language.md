# Operator-facing state language

## Outcome

Make every primary shell surface speak one operator-facing status language built from current state, cause, and next action.

## Why this direction exists

The product already has the right runtime capabilities, but the visible status model still mixes operator guidance with internal implementation terms. That makes pause, invalid, and resume states harder to trust than they should be.

## Long-horizon plan

- standardize the primary state vocabulary across shell, planning, queue, automation, and resumed-session surfaces
- make blocked and paused states explicitly recoverable
- keep compact status copy aligned with richer overlay explanations
- reserve planner and protocol detail for secondary debug or notice lines

## Near-term bias

- fix the shared state taxonomy first
- then align pause and repair wording
- then clean up resumed-session and planning recovery copy that still uses older labels

## Relevant inputs

- `docs/plan/14-product-elevation-blueprint.md`
- `docs/plan/15-ux-flow-rearchitecture.md`
- `src/application/service/planning_runtime_policy_service.rs`
- `src/adapter/inbound/tui/app/planning/status_projection.rs`
- `src/adapter/inbound/tui/app/shell_presentation/status_panels.rs`

## Task derivation guidance

- derive slices that change one family of operator-facing messages at a time
- prefer shared projection seams before touching overlay-specific copy
- keep resumed-session language on the same vocabulary as normal shell status

## Avoid

- changing automation policy semantics in the same slice unless copy cannot improve otherwise
- introducing new planner-only terms into primary status lines
