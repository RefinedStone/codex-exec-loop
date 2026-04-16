# Planning workspace lifecycle commands

## Outcome

Let the operator inspect, initialize, and safely reset planning state from the workspace root or from inside the shell without learning two different planning models.

## Why this direction exists

Long-lived self-host use gets fragile when planning health can only be checked after entering the TUI, or when bootstrap and reset behavior live behind internal-only flows. The product needs a stable lifecycle command surface before `akra-queue` can be trusted to keep improving the same workspace for many hours.

## Long-horizon plan

- ship a read-only planning health report through external `akra doctor` and in-shell `:doctor`
- expose the default scaffold through `akra init` and a fast in-shell `:init` entrypoint
- define safe reset targets for `queue`, `directions`, and `all` through external `akra reset` and in-shell `:reset`
- keep all lifecycle commands backed by shared application services instead of duplicated shell-specific rules

## Near-term bias

- start with external `akra doctor` because pre-launch planning visibility is the most direct trust unlock
- then add shared bootstrap support for `akra init`
- add reset semantics only after rewritten artifacts, refusal rules, and confirmation paths are explicit

## Relevant inputs

- `docs/plan/14-product-elevation-blueprint.md`
- `docs/plan/16-planning-and-automation-evolution.md`
- `docs/plan/18-planning-workspace-lifecycle-commands.md`
- `src/application/service/planning_validation_service.rs`
- `src/application/service/planning_bootstrap_service.rs`
- `src/application/service/planning_runtime_facade_service.rs`

## Task derivation guidance

- derive one reviewable slice per command family or shared lifecycle service seam
- prefer shared validation, bootstrap, and reset primitives before wiring multiple entrypoints
- keep queue tasks explicit about which artifacts are inspected, created, or rewritten

## Avoid

- creating CLI-only planning logic that diverges from the in-shell contract
- bundling doctor, init, and reset into one oversized task
- making reset behavior implicit or ambiguous about destructive scope
