# Operator-facing state language

## Outcome

Make the shell read like a dependable execution cockpit by standardizing operator-facing state language around:

- current state
- cause
- next action

## Why this direction exists

The product already has strong runtime, planning, and repair features. The main drag on long-lived use is that many states still read like internal implementation status instead of operator guidance.

## Long-horizon plan

- normalize shared vocabulary across shell summaries, queue inspection, automation controls, and planning recovery
- separate compact shell copy from full overlay explanations
- make pause and blocked states explicitly recoverable
- keep debug detail optional instead of mixing it into the primary status language

## Near-term bias

- start with the shell footer and compact status lines
- then fix automation pause and queue wording
- then align planning invalid and repair messaging

## Relevant inputs

- `docs/plan/14-product-elevation-blueprint.md`
- `docs/plan/15-ux-flow-rearchitecture.md`
- `src/adapter/inbound/tui/app/shell_presentation.rs`
- `src/adapter/inbound/tui/app/shell_presentation/status_panels.rs`
- `src/adapter/inbound/tui/app/conversation_model/auto_follow.rs`

## Task derivation guidance

- derive small slices that improve one family of operator-facing messages at a time
- prefer copy and projection seams before policy changes
- keep the same vocabulary across shell, queue, planning, and automation once introduced

## Avoid

- changing automation policy semantics in the same slice unless the copy fix is impossible otherwise
- adding new raw planner terminology to the primary shell surface
