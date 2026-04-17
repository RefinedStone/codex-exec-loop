# Supersession mode entry and readiness

## Outcome

Make parallel mode enter as a supervisor-first shell state that can explain whether supersession is
ready, degraded, blocked, or repairing before any agent is launched.

## Why this direction exists

The supersession docs define parallel mode as an explicit operating mode, not a background feature.
The first visible product contract is therefore mode entry and readiness: the shell has to decide
whether it can become a control tower in the current workspace and explain that decision clearly.

## Long-horizon plan

- add an explicit parallel-mode toggle and status surface
- route `:sessions` and related shell summaries to supersession only when parallel mode is on
- surface git, planning, push, and GitHub capability readiness as operator-facing state
- keep degraded readiness recoverable instead of treating missing capabilities as fatal startup errors

## Near-term bias

- land capability detection and mode routing before agent launch
- keep degraded-state copy explicit and recoverable from the first slice
- keep normal-mode session browsing intact while parallel mode is off

## Relevant inputs

- `docs/supersession/01-product-model.md`
- `docs/supersession/02-operator-mode-and-shell-model.md`
- `docs/supersession/08-capabilities-degraded-mode-and-failures.md`
- `docs/supersession/10-implementation-slices.md`
- `src/adapter/inbound/tui/app/shell_controller.rs`
- `src/adapter/inbound/tui/app/shell_presentation.rs`
- `src/adapter/inbound/cli.rs`

## Task derivation guidance

- derive slices around one mode-entry or capability family at a time
- keep current state, cause, and next action visible in compact shell summaries
- prefer adding a durable readiness snapshot over one-off command checks

## Avoid

- launching agent sessions before mode-entry and degraded-state rules are clear
- replacing the normal session browser outside parallel mode
