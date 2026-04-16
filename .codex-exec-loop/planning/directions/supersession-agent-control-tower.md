# Supersession agent control tower

## Outcome

Replace the single-main-session mental model with a control-tower surface that tracks multiple
main-grade agent sessions, their assignments, durations, and completion milestones.

## Why this direction exists

Supersession is not a hidden worker fan-out. It is a supervisor that manages several real
implementation agents. The operator therefore needs one surface that can explain what each agent is
doing, which task it owns, and whether its result is only reported or already official.

## Long-horizon plan

- add supervisor snapshots for active agents, selected-agent detail, and completion feed
- represent agent lifecycle from requested through cleaned
- show running duration, task ownership, branch identity, and latest summary in one board
- keep supervisor operational and non-conversational

## Near-term bias

- ship the lifecycle model and one control-tower board before richer actions
- keep completion feed distinct from merge queue state
- make `reported_complete` and official completion visibly different

## Relevant inputs

- `docs/supersession/01-product-model.md`
- `docs/supersession/02-operator-mode-and-shell-model.md`
- `docs/supersession/03-agent-session-lifecycle.md`
- `docs/supersession/07-supervisor-ui-and-surfaces.md`
- `src/adapter/inbound/tui/app/session_overlay_ui.rs`
- `src/adapter/inbound/tui/app/shell_presentation.rs`
- `src/application/service/session_service.rs`

## Task derivation guidance

- derive slices around one operator-visible board or lifecycle family at a time
- keep selected-agent detail narrow enough that each slice stays reviewable
- prefer explicit lifecycle state names over inferred activity from raw stream events

## Avoid

- treating the supervisor as another chat thread
- hiding agent identity or task ownership behind debug-only output
