# Execution Order

## Phase 1: Lock The Baseline
1. Keep the current streaming shell behavior working.
2. Update docs and terminology to match `prerelease`.
3. Do not regress already-landed features such as auto follow-up, workspace templates, stop rules, new-thread flow, or streamed turn handling while making UX changes.

## Phase 2: Reduce Page-Style Navigation
1. Rework `Home` and `SessionList` so the shell feels more primary.
2. Preserve startup diagnostics and recent-session browsing, but make them lighter-weight.
3. Avoid introducing a new route tree unless it clearly simplifies the shell flow.

## Phase 3: Improve Runtime Lifecycle
1. Introduce a longer-lived runtime abstraction behind the outbound port.
2. Keep protocol parsing and item mapping inside adapters.
3. Feed runtime events into the existing UI loop through typed messages.

## Phase 4: Shell Ergonomics
1. Improve input handling and status visibility.
2. Add more operator-friendly template and stop-rule controls.
3. Keep transcript and activity rendering incremental and predictable.

## Phase 5: Hardening
1. Add regression tests around streamed event handling and auto follow-up behavior.
2. Add failure-path coverage for transport exit and malformed payloads.
3. Reassess module splits inside `adapter/inbound/tui` after the lifecycle work lands.

## Delivery Rule
Prefer changes that preserve the current live shell while improving continuity. The branch already crossed the "placeholder shell" stage, so future work should extend the current runtime instead of replacing it wholesale.
