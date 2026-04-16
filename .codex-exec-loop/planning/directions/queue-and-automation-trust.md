# Queue and automation trust

## Outcome

Turn queue-driven continuation into a system the operator can trust because it is legible.

## Why this direction exists

Queue-driven automation is the product differentiator. Its next maturity step is not more hidden autonomy, but clearer explanation of what work is current, what work is optional, and why automation continued or paused.

## Long-horizon plan

- make queue inspection read like a work board: now, next, candidates, blocked or skipped
- make automation controls the place where policy, preview, pause reason, and resume path meet
- translate repeated queue head, queue idle, and invalid planning into operator-language recovery states
- keep proposal handling visible and intentional instead of planner-only residue

## Near-term bias

- reframe queue overlay information architecture
- rework pause and stop copy in automation surfaces
- keep one clear actionable queue head and leave alternates as proposals

## Relevant inputs

- `docs/plan/14-product-elevation-blueprint.md`
- `docs/plan/15-ux-flow-rearchitecture.md`
- `docs/plan/16-planning-and-automation-evolution.md`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/queue.rs`
- `src/application/service/planning_runtime_policy_service.rs`
- `src/application/service/planning_runtime_facade_service.rs`

## Task derivation guidance

- prefer one reviewable slice per queue or automation behavior family
- keep explanations tied to operator actions, not internal guard names
- when multiple follow-up ideas exist, promote one executable slice and leave the rest as proposals

## Avoid

- changing queue semantics and presentation semantics in one oversized task
- inventing additional automation power without also improving recovery clarity
