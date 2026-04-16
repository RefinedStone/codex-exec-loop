# Automation recovery language

## Outcome

Turn automation controls into the surface that explains what happened, why, and how the operator resumes progress.

## Why this direction exists

Automation is the product differentiator, but the current controls still read like a settings and debug panel with recovery details attached. Operators need the opposite emphasis: pause reason and resume path first, knobs second.

## Long-horizon plan

- explain continue, pause, and stop outcomes in one consistent operator language
- map each blocked or paused automation state to one primary recovery action or surface
- keep planner debug detail available without making it the main story
- align automation copy with the same state taxonomy used elsewhere in the shell

## Near-term bias

- improve pause and stop reasons before adding new automation behavior
- surface recovery paths next to the reason they matter
- keep configuration details secondary to runtime explanation

## Relevant inputs

- `docs/plan/14-product-elevation-blueprint.md`
- `docs/plan/15-ux-flow-rearchitecture.md`
- `src/adapter/inbound/tui/app/planning/presentation.rs`
- `src/application/service/planning_runtime_policy_service.rs`
- `src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution.rs`

## Task derivation guidance

- derive slices around one pause or recovery family at a time
- keep automation status, preview, and recovery copy logically connected
- prefer operator-action wording over internal guard or policy names

## Avoid

- expanding automation power without also improving pause and recovery explanation
- making planner debug output the primary explanation channel
