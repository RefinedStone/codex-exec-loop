# native

Rust TUI client prototype for `codex-exec-loop`.

This crate is the main product track for `codex-exec-loop`.

Current milestones:

- spawn `codex app-server` over stdio
- perform `initialize`
- check account/auth state
- render a startup dashboard
- load recent sessions with `thread/list`
- load selected thread history with `thread/read`
- resume a selected thread and send a prompt with `turn/start`
- stream agent message deltas into the shell view
- continue work automatically with a builtin next-task follow-up prompt
- switch builtin follow-up strategies from the TUI

Protocol shape is pinned with a checked-in schema snapshot under `schema/`.

## Architecture

The native crate prefers a Spring Boot Kotlin style hexagonal layout.

- `domain`
- `application/service`
- `application/port`
- `adapter/inbound`
- `adapter/outbound`
