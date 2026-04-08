# TODO

## Product Direction

- Primary product: Rust native client on top of `codex app-server`
- Python CLI: legacy compatibility path only
- No new feature work should start on the Python CLI unless it is required to unblock native parity or migration

## Current Focus

- Make native the real "agent loop" product, not just a conversation shell
- Finish the inline terminal-flow reset by removing repeated full-frame redraw from main-buffer mode
- Run real terminal validation on macOS and Windows from the published matrix
- Land only focused Windows compatibility fixes when the validation matrix produces concrete findings
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
  - validation result capture helpers
  - validation record directory
  - validation coverage summary helper
  - validation markdown report helper
  - packaging runbook
  - release checksum helpers
- inline shell parity
  - inline inspection surfaces for diagnostics, recent sessions, and follow-up templates
  - stable streaming-history buffering for inline mode, with live output kept separate from committed transcript history and lifecycle markers committed into stable history
  - inline shell chrome collapsed toward one tail prompt region, with transcript pinned to tail, compact prompt guidance, and no dedicated tail title row
- migration docs
  - repository root README now presents native as the main product path
  - Python CLI instructions are reduced to compatibility guidance

## Next

- remove the remaining inline repeated redraw path so host terminal scrollback stops replaying shell frames
- keep moving inline mode toward top-to-bottom terminal flow where host terminal scroll is the primary history mechanism
- run real terminal validation on macOS and Windows and land only focused compatibility fixes when findings exist

## Migration

- keep Python CLI as a compatibility path until its final removal plan is explicit
