# Inline Scrollback Shell

This file is the compact current contract for the inline shell on `prerelease`.

## Durable Facts
- inline mode is the default frontend; alternate-screen is explicit opt-in
- `thread_id` and `turn_id` are protocol values from `codex app-server`, not native-client-generated UI ids
- committed transcript history is appended into host terminal scrollback separately from tail updates
- startup ASCII art, when enabled, persists in scrollback instead of behaving like a transient frame
- hidden inline mode uses one fixed viewport budget with the tail rendered from the top of the visible viewport

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
- blank-draft startup reads as startup context, conversation placeholder, and prompt
- inline mode keeps one live region for prompt, transient streaming, and inspections
- once committed history exists, the live region starts at the first visible row of its viewport so it stays attached to the latest scrollback line
- inline status behaves like a short notice band, not a permanent heavy footer
