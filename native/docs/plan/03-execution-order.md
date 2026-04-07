# Execution Order

## Phase 1: Lock The Baseline
1. Keep the current streaming shell behavior working.
2. Update docs and terminology to match `prerelease`.
3. Do not regress already-landed features such as auto follow-up, workspace templates, stop rules, new-thread flow, or streamed turn handling while making UX changes.

## Phase 2: Commit To A Stream-First Shell
1. Treat the transcript as the primary vertical flow.
2. Keep the composer anchored at the bottom instead of centering the experience around panels.
3. Avoid introducing a new route tree unless it clearly helps the stream-first shell.

## Phase 3: Reduce Full-Screen TUI Dependence
1. Rework or remove alternate-screen assumptions that block a scrollback-native experience.
2. Preserve startup diagnostics and recent-session browsing, but make them secondary to the shell flow.
3. Prefer lightweight overlays, command-style entry, or inline events over always-visible side panels.

## Phase 4: Improve Runtime Lifecycle
1. Introduce a longer-lived runtime abstraction behind the outbound port.
2. Keep protocol parsing and item mapping inside adapters.
3. Feed runtime events into the existing UI loop through typed messages.

## Phase 5: Stream Shell Ergonomics
1. Improve input handling and fixed-composer behavior.
2. Add more operator-friendly template and stop-rule controls.
3. Keep transcript rendering append-only and predictable.

## Phase 6: Hardening
1. Add regression tests around streamed event handling and auto follow-up behavior.
2. Add failure-path coverage for transport exit and malformed payloads.
3. Reassess module splits inside `adapter/inbound/tui` after the stream-first lifecycle work lands.

## Delivery Rule
Prefer changes that preserve the current live shell while moving it toward a scrollback-native CLI. The branch already crossed the "placeholder shell" stage, so future work should extend the current runtime, but the current full-screen TUI layout should be treated as transitional rather than final.
