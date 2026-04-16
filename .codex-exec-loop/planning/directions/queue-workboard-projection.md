# Queue workboard projection

## Outcome

Make queue inspection feel like a readable work board instead of a thin view over planner internals.

## Why this direction exists

The runtime already computes enough queue structure to show current work, follow-up work, proposals, and blocked items. The overlay still presents that structure as mixed queue, proposal, and note fragments, which slows operator understanding.

## Long-horizon plan

- project queue state as now, next, proposed, and blocked work
- keep one actionable queue head and make alternate work visibly secondary
- surface blocked or skipped work as work framing rather than planner residue
- reuse the same queue framing in resumed-session and compact shell summaries

## Near-term bias

- rework the queue overlay information architecture first
- then align summary lines and note lines with the same framing
- keep blocked visibility readable without opening raw planning files

## Relevant inputs

- `docs/plan/14-product-elevation-blueprint.md`
- `docs/plan/15-ux-flow-rearchitecture.md`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/queue.rs`
- `src/adapter/inbound/tui/app/planning/status_projection.rs`
- `src/application/service/planning_prompt_service.rs`

## Task derivation guidance

- prefer slices that improve one queue section at a time
- keep queue framing consistent between overlay, footer, and resumed-session summary
- treat blocked-work visibility as a first-class section, not an overflow note

## Avoid

- merging queue semantics changes with large planner policy changes
- hiding blocked or skipped work behind debug-only affordances
