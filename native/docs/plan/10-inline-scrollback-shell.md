# Inline Scrollback Shell

This is the compact source of truth for the remaining inline-shell work.

The goal is still one thing:
- inline mode should read like terminal flow, not like a fullscreen frame being replayed in the main buffer

## Current Stance

Landed foundation on `prerelease`:
- explicit inline vs alternate-screen frontend split
- inline inspection surfaces for diagnostics, sessions, and follow-up templates
- compact tail prompt guidance
- scrollback-safe history buffering that separates live output from committed history

Still open:
- inline mode still repaints committed history through one Ratatui frame in some terminals
- the active prompt does not yet own an explicit visible cursor in the inline path
- streamed agent output is still too easy to miss because the live region behaves more like summary/status chrome than a true visible stream
- completion and auto-follow copy still exposes raw turn ids and keeps too much weight in the bottom status surface

## Durable Facts

- `thread_id` and `turn_id` are protocol values from `codex app-server`, not native-client-generated UI ids. The outbound adapter forwards `Thread.id` and `Turn.id` into shell state.
- the current noisy strings such as `turn completed: <id>` are native-client presentation copy layered on top of those ids
- the inline frontend currently restores the terminal cursor on exit, but it does not set a focused prompt cursor during normal inline rendering
- the runtime already receives streamed agent deltas, but the inline shell still compresses too much of that activity into status-oriented output

## UX Contract

### 1. Prompt Contract

- the active prompt must show a visible cursor in the prompt surface
- cursor blinking is preferred when the host terminal allows it, but visible focus is the required contract
- non-input surfaces such as inspections, passive status notices, and completed transcript history should not pretend to own the cursor

### 2. Streaming Contract

- while a turn is running, the operator should see agent text change before completion
- status-only activity is not a substitute for visible streamed content
- once the turn completes, the final assistant text should commit into normal scrollback history and the live region can shrink again

### 3. Status Contract

- the default inline status surface should be compact and flow-oriented
- raw `thread_id` and `turn_id` values should stay in state for routing, correlation, and debugging, but remain hidden from routine inline status copy
- values that only help debugging should move to explicit inspection or debug surfaces instead of occupying the default tail region

### 4. Layout Contract

- host terminal scrollback is the primary history surface in inline mode
- inline mode keeps one tail-anchored live region for prompt, transient streaming, and inspections
- inline status should behave more like a short terminal flow box or notice band than a permanently heavy footer

## Execution Checkpoints

1. Prompt cursor and focus ownership
- set cursor position from the active prompt render path
- keep cursor ownership out of passive inspection and transcript rendering

2. Visible streaming live region
- render actual agent delta text in the active live region
- keep buffered input intact while the turn is still running

3. Compact status and id elision
- split operator-facing status copy from raw correlation ids
- remove raw turn ids from routine completion and auto-follow messages
- reduce persistent footer weight where a transient flow notice is enough

4. Final redraw elimination
- stop repainting committed history as one shared frame
- keep tail updates local so prior output stays visually stable in scrollback

## Done When

- inline mode no longer looks like a replayed fullscreen frame
- the active prompt owns a visible cursor
- agent text visibly streams before completion
- default status copy hides raw ids and stays compact
- the validation matrix passes the prompt, stream, and status checks added for this workstream
