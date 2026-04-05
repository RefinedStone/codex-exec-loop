# TODO

## Product Direction

- Primary product: Rust native client on top of `codex app-server`
- Python CLI: legacy compatibility path only
- No new feature work should start on the Python CLI unless it is required to unblock native parity or migration

## Current Focus

- Make native the real "agent loop" product, not just a conversation shell
- Run canned auto-follow-up prompts after turn completion
- Keep the flow understandable for a Spring Boot Kotlin developer

## Done

- startup dashboard and environment checks
- recent session list via `thread/list`
- existing session resume and new thread start
- streamed conversation rendering
- conversation tail visibility for long histories
- Ctrl+C back navigation and exit confirmation
- native auto-follow-up v1 with builtin next-task prompt and per-conversation toggle
- native builtin follow-up strategy picker
  - next-task
  - plan-queue
  - bugfix
  - docs

## Next

- support external prompt/template files instead of builtin template only
- add stop rules for native auto-follow-up
  - keyword stop
  - no-file-change stop
  - max-auto-turns editing from UI
- show clearer activity for auto follow-up queue / submit / stop decisions
- add session search, paging, and recent project filters
- add approval and tool activity panels
- add GitHub PR review/comment polling in the native UI
- validate packaging and terminal behavior on macOS and Windows

## Migration

- deprecate Python CLI usage in README once native reaches follow-up/template parity
- remove Python CLI from the main product story after native covers the required workflow
