# Supervisor UI And Surfaces

This document defines the target supersession model, not shipped behavior.

## UI Goal

The supersession surface must let the operator understand and steer a fleet of agent sessions
without opening multiple unrelated overlays or reading raw git/process internals.

## Primary Panels

| Panel | Must answer |
| --- | --- |
| capability panel | can supersession safely run here right now? |
| pool board | how much execution capacity is left? |
| active agents list | which task is each agent working on? |
| completion feed | what finished recently, and is it official yet? |
| merge queue panel | what is integrating now, and what is blocked next? |
| alerts panel | what needs operator action immediately? |
| selected-agent detail | what exactly happened in one slot, branch, and session? |

## Default Layout

The intended layout is a two-column board:

- left column
  - capability panel
  - pool board
  - active agents list
- right column
  - selected-agent detail
  - completion feed
  - merge queue

Alerts may render above both columns or in a persistent footer depending on terminal width.

## Compact Shell Summary

Even when the overlay is closed, the main shell summary should still show:

- mode: normal or parallel
- pool summary: idle/running/blocked counts
- merge queue summary: current head state or idle
- top alert if one exists

This keeps supersession visible from the normal shell tail.

## Capability Panel

Shows:

- git repo readiness
- `akra` branch readiness
- push readiness
- `gh` binary readiness
- `gh` auth readiness
- planning readiness

Each line uses explicit state wording and an action-oriented explanation when degraded.

## Pool Board

Shows:

- configured pool size
- slot states
- exhausted indicator when no idle slots remain
- blocked slot count
- last reconcile timestamp

Slot rows should include slot id, branch, worktree label, and current owner when leased.

## Active Agents List

Each row should include:

- agent id
- assigned task title
- slot id
- branch name
- state label
- running duration
- latest summary excerpt

Rows should be sortable by recency or state, but the default ordering should prioritize running
and failed agents above idle history.

## Completion Feed

The feed must distinguish:

- `reported`
- `ledger refreshing`
- `official`
- `merge queued`
- `merged`

This prevents the operator from confusing agent self-report with official task advancement.

## Merge Queue Panel

Each item should show:

- source agent
- task title
- queue state
- branch
- commit short sha
- most recent integration note

The queue head should be visually obvious.

## Selected-Agent Detail

When the operator selects an agent or slot, the detail pane shows:

- task id and title
- thread/session id
- worktree path
- branch name
- lease start time
- latest summary
- validation summary
- ledger refresh outcome
- distributor outcome if present

## Alert Rules

Alerts should be reserved for cases that block or materially change the next operator action:

- pool exhausted
- planning invalid
- ledger refresh failed
- push unavailable
- `gh` unavailable
- merge conflict
- cleanup verification failed

Alert copy should follow:

- current state
- cause
- next action

## Related Docs

- [02-operator-mode-and-shell-model.md](02-operator-mode-and-shell-model.md)
- [03-agent-session-lifecycle.md](03-agent-session-lifecycle.md)
- [08-capabilities-degraded-mode-and-failures.md](08-capabilities-degraded-mode-and-failures.md)
- [../design/02-tui-shell-flow.md](../design/02-tui-shell-flow.md)

## Code Impact

Expected entrypoints:

- `src/adapter/inbound/tui/app/shell_presentation.rs`
- `src/adapter/inbound/tui/app/session_overlay_ui.rs`
- `src/adapter/inbound/tui/app/shell_controller.rs`
