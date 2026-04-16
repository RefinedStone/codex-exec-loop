# Operator Mode And Shell Model

This document defines the target supersession model, not shipped behavior.

## Goal

Parallel mode should replace the operator's mental model of "one shell equals one main session"
with "one shell equals one control tower" while preserving a clear boundary with current normal mode.

## Top-Level Operating Modes

| Mode | What the operator sees | Primary job |
| --- | --- | --- |
| normal mode | today's conversation-first shell | work in one main session |
| parallel mode | supersession control tower | coordinate multiple agent sessions |

Normal mode remains the default. Parallel mode is an explicit opt-in state.

## Parallel Mode Shell States

| State | Meaning | Required operator answer |
| --- | --- | --- |
| prepare | capability checks, pool reconcile, planning readiness | can supersession start safely here? |
| supervise | agent board, merge queue, and live status | what is each agent doing right now? |
| inspect | agent detail, completion feed, capability detail | what does the system currently know? |
| recover | degraded readiness, blocked merge, failed cleanup | what is the safest next action? |

These are operator-facing states. Internal reducers may still use finer-grained state.

## Session Entry Contract

| Surface | Normal mode | Parallel mode |
| --- | --- | --- |
| startup shell | opens a blank draft or resumed thread | opens supersession prepare state |
| `:sessions` | recent sessions browser | supersession board |
| session identity | current thread id and title | supersession id, active agent count, pool status |
| main action | type and submit | assign, inspect, merge, recover |

Parallel mode does not delete the concept of recent sessions, but it demotes it to supporting
information rather than the primary entry surface.

## Command Surface

| Command | Role in normal mode | Role in parallel mode |
| --- | --- | --- |
| `:parallel on` | enable supersession if readiness allows | no-op if already enabled |
| `:parallel off` | no-op if already disabled | stop supersession surface and return to normal shell |
| `:parallel` | inspect readiness or current state | inspect current supersession summary |
| `:sessions` | open recent sessions | open supersession board |
| `:planning` | planning authoring and review | planning authoring for ledger authority and recovery |
| `:queue` | queue inspection | queue inspection derived from ledger after agent refresh |

## What Supersession Must Always Answer

- Is parallel mode actually ready, degraded, or blocked?
- How many slots are idle, running, blocked, or unavailable?
- Which task is assigned to which agent right now?
- Which agent results are only reported versus officially ledger-applied?
- What is the next blocking issue: planning, git, GitHub, merge queue, or cleanup?

## Surface Ownership

### Compact Shell Summary

Must always carry:

- mode label
- pool summary
- running agent count
- merge queue head summary
- top blocking alert when one exists

### Supersession Overlay

Owns:

- capability panel
- pool board
- active agent list
- completion feed
- merge queue
- selected-agent detail

### Planning Overlay

Owns:

- ledger health
- directions and queue authoring
- recovery when hidden planning worker fails

It remains the authoring surface, not the agent board.

## Copy Principles

- Use explicit state words: `ready`, `running`, `paused`, `blocked`, `repairing`, `degraded`.
- Distinguish `reported` from `official`.
- Phrase recovery as operator verbs: `retry capability check`, `reconcile pool`, `review queue`, `repair planning`, `resume distributor`.
- Avoid describing internal protocol or process details unless the operator can act on them directly.

## Mode Switching Rules

- Enabling parallel mode performs capability and planning readiness checks first.
- If checks partially fail, enter supersession in degraded state rather than crashing the app.
- Disabling parallel mode returns shell ownership to the normal conversation surface without deleting supersession records.
- Parallel mode never silently mutates current normal-mode conversation history.

## Related Docs

- [01-product-model.md](01-product-model.md)
- [07-supervisor-ui-and-surfaces.md](07-supervisor-ui-and-surfaces.md)
- [08-capabilities-degraded-mode-and-failures.md](08-capabilities-degraded-mode-and-failures.md)
- [../design/02-tui-shell-flow.md](../design/02-tui-shell-flow.md)

## Code Impact

Expected entrypoints:

- `src/adapter/inbound/tui/app/shell_controller.rs`
- `src/adapter/inbound/tui/app/shell_presentation.rs`
- `src/adapter/inbound/tui/app/session_overlay_ui.rs`
