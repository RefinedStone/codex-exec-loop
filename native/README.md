# native

Rust TUI client prototype for `codex-exec-loop`.

This crate is the start of the `codex app-server` based client. The current
milestones are intentionally small and incremental:

- spawn `codex app-server` over stdio
- perform `initialize`
- check account/auth state
- render a startup dashboard
- load recent sessions with `thread/list`
- open a placeholder shell screen for the selected session

Protocol shape is pinned with a checked-in schema snapshot under `schema/`.

## Architecture

The native crate prefers a Spring Boot Kotlin style hexagonal layout.

- `domain`
- `application/service`
- `application/port`
- `adapter/inbound`
- `adapter/outbound`
