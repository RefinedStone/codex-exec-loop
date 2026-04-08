# TODO

## Product Direction

- Primary product: Rust native client on top of `codex app-server`
- Python CLI: legacy compatibility path only
- No new feature work should start on the Python CLI unless it is required to unblock native parity or migration

## Current Focus

- Make native the real "agent loop" product, not just a conversation shell
- Make inline mode read like natural terminal history instead of a fullscreen frame replay
- Run canned auto-follow-up prompts after turn completion
- Keep the flow understandable for a Spring Boot Kotlin developer

## Done

- shell baseline
  - startup dashboard and environment checks
  - recent session list via `thread/list`
  - existing session resume and new thread start
  - streamed conversation rendering
  - conversation tail visibility for long histories
  - Ctrl+C back navigation and exit confirmation
- automation controls
  - builtin auto-follow-up toggle and strategy picker
  - workspace follow-up template loading and reload
  - editable max auto turns
  - startup-pending manual submit queue
  - stop keyword and no-file-change stop rules
  - clearer queue / submit / stop / skip activity summaries
- session browser
  - search query
  - paging
  - recent-project filter
  - keyboard controls and result shaping
- runtime and operator visibility
  - shared runtime request policy
  - approval and tool activity status
  - reconnect / reset / warning visibility
  - GitHub PR review polling and review-change notices
- platform and packaging docs
  - validation matrix for terminal behavior
  - packaging runbook
  - release checksum helpers
- inline shell parity
  - inline inspection surfaces for diagnostics, recent sessions, and follow-up templates

## Next

- make streaming output scrollback-safe without replaying the whole shell frame
- run real terminal validation on macOS and Windows and land only focused compatibility fixes when findings exist

## Migration

- deprecate Python CLI usage in README once native reaches follow-up/template parity
- remove Python CLI from the main product story after native covers the required workflow
