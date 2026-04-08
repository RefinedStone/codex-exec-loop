# Known Gaps And Risk Areas

## Current Gaps
- inline mode is closer to the target "flow shell" contract, but the shell still runs as a raw-mode tail region rather than a fully append-only CLI history
- real terminal validation is still missing on macOS and Windows, so Windows fixes should stay conditional on recorded findings
- recent-session loading and blocked startup still gate shell actions, even though manual prompt submission can now queue while startup checks are running
- the shared runtime is better than the old action-scoped model, but concurrent requests still need a fallback path while a streaming turn holds the shared runtime
- `src/adapter/inbound/tui/app.rs` is much smaller than before, but future work should keep it near composition and shared shell state rather than grow it again
- input and long-session ergonomics are still limited compared with a mature CLI shell

## Risk Rule
Do not restart from a blank-shell rewrite. The main missing work is lifecycle and ergonomics, not missing protocol coverage.

## Documentation Rule
When adding new docs, preserve the gaps above only if they still affect phase-2 decisions. Avoid turning this file into a rolling bug list.
