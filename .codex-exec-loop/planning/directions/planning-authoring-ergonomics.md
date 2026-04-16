# Planning authoring ergonomics

## Outcome

Make planning feel like the lightest reliable way to keep a workspace moving.

## Why this direction exists

The planning contract is already powerful, but first-run setup still feels heavier than it should. Long-horizon automation only works if operators can enter planning quickly and still trust what became active.

## Long-horizon plan

- make simple mode the obvious happy path
- make staged review emphasize what becomes active and what behavior it enables
- keep directions maintenance and queue-idle prompt maintenance feeling like part of planning authoring
- preserve explicit promotion and accepted-state trust even while smoothing the flow

## Near-term bias

- improve simple mode review copy
- clarify queue-idle policy during setup
- reduce friction in directions maintenance for supporting files

## Relevant inputs

- `docs/plan/14-product-elevation-blueprint.md`
- `docs/plan/16-planning-and-automation-evolution.md`
- `src/adapter/inbound/tui/app/planning_init_overlay_ui.rs`
- `src/adapter/inbound/tui/app/planning/controller.rs`
- `src/application/service/planning_directions_service.rs`

## Task derivation guidance

- prefer slices that improve first successful planning loops
- treat accepted planning as the trustworthy source of automation behavior
- keep planning files explicit even when simplifying the operator path

## Avoid

- reopening guided or LLM-assisted authoring before simple and manual paths are clearly strong
- collapsing staging and promotion into implicit behavior
