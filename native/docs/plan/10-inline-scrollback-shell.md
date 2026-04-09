# Inline Scrollback Shell

This file is the compact current contract for the inline shell on `prerelease`.

## Durable Facts
- inline mode is the default frontend; alternate-screen is explicit opt-in
- `thread_id` and `turn_id` are protocol values from `codex app-server`, not native-client-generated UI ids
- committed transcript history is appended into host terminal scrollback separately from tail updates
- hidden inline mode uses one fixed viewport budget with a bottom-anchored tail

## Prompt Contract
- the active prompt shows a visible cursor in the prompt surface
- cursor blinking is preferred when the host terminal allows it, but visible focus is the required contract
- passive transcript or inspection surfaces do not own the cursor

## Streaming Contract
- while a turn is running, operator-visible agent text changes before completion
- buffered input stays intact during streaming
- when the turn completes, the final assistant text moves into normal scrollback history and the live region can shrink

## Status Contract
- default inline status stays compact and flow-oriented
- routine inline copy hides raw `thread_id` and `turn_id`
- debug-only identifiers belong in explicit inspection or debug surfaces, not in routine shell copy

## Layout Contract
- host terminal scrollback is the primary history surface
- inline mode keeps one tail-anchored live region for prompt, transient streaming, and inspections
- inline status behaves like a short notice band, not a permanent heavy footer
