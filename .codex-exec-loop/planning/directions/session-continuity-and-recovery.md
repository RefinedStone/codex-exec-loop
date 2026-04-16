# Session continuity and recovery

## Outcome

Make returning to work feel like returning to the current execution context, not merely reopening transcript history.

## Why this direction exists

The product is optimized for long-lived solo sessions. That value weakens if resumed sessions do not immediately reveal whether planning is active, what remains in the queue, and what recovery path applies.

## Long-horizon plan

- surface planning state and queue summary as part of resumed context
- map each blocked or paused state to one primary recovery surface
- reduce the need to open multiple overlays to understand where work stopped
- support all-day usage by shrinking the cost of context reconstruction

## Near-term bias

- improve resumed shell summaries
- separate diagnostics, planning recovery, and queue recovery more clearly
- keep current session identity and work identity visible together

## Relevant inputs

- `docs/plan/14-product-elevation-blueprint.md`
- `docs/plan/15-ux-flow-rearchitecture.md`
- `src/adapter/inbound/tui/app/session_overlay_ui.rs`
- `src/application/service/session_service.rs`
- `src/adapter/inbound/tui/app/conversation_lifecycle.rs`

## Task derivation guidance

- prefer slices that reveal state earlier after resume
- tie recovery improvements to one visible operator question at a time
- keep continuity focused on both conversation state and work-management state

## Avoid

- adding large new session-management features before the current resume path is legible
- mixing unrelated queue, planning, and diagnostics changes into one task
