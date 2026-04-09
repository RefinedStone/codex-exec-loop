# Inline Scrollback Shell

This is the compact source of truth for the landed inline-shell contract and the remaining validation work.

The goal is one thing:
- inline mode should read like terminal flow, not like a fullscreen frame being replayed in the main buffer

## Current Stance

Landed on `prerelease`:
- explicit inline vs alternate-screen frontend split
- inline inspection surfaces for diagnostics, sessions, and follow-up templates
- compact tail prompt guidance
- scrollback-safe history buffering that separates live output from committed history
- fixed inline viewport budget with a bottom-anchored hidden tail so normal inline updates no longer rebuild the viewport height
- visible prompt cursor ownership in the inline path
- visible streamed agent text before completion
- compact routine status copy without raw turn ids

Still open:
- real terminal validation on macOS and Windows
- follow-on ergonomics only if validation finds a concrete issue

## Durable Facts

- `thread_id` and `turn_id` are protocol values from `codex app-server`, not native-client-generated UI ids. The outbound adapter forwards `Thread.id` and `Turn.id` into shell state.
- the current noisy strings such as `turn completed: <id>` are native-client presentation copy layered on top of those ids
- the inline frontend now keeps one fixed viewport budget in inline mode and appends committed history into scrollback separately from tail redraw
- the runtime already carried streamed deltas and protocol ids before this pass; the inline frontend now presents them through operator-facing copy instead of routine raw-id status strings

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

## Landed Checkpoints

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
- keep the inline viewport height fixed in hidden mode
- keep committed history append-only in scrollback while tail updates remain local

## Remaining Validation

- confirm the fixed hidden-tail viewport no longer feels replay-like in real terminals
- pass the prompt, stream, status, and scrollback checks in the validation matrix
