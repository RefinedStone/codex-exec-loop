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
- load workspace follow-up templates from `.codex-exec-loop/followups/`
- stop auto follow-up when the agent emits `AUTO_STOP` or when the no-file-change rule is enabled
- schedule optional GitHub PR review/comment polling for one configured pull request

Protocol shape is pinned with a checked-in schema snapshot under `schema/`.

## Optional GitHub Polling

The runtime can poll one GitHub pull request in the background and keep the polling state in the shell footer.

- `CODEX_EXEC_LOOP_GITHUB_PR=owner/repo#123`: enable polling for a specific pull request
- `CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS=60`: optional polling interval in seconds; defaults to `60`

When the poller is enabled, the footer shows whether polling is starting, running, watching normally, or blocked by a setup/runtime error. Review-change notices are still a later UI slice.

## Architecture

The native crate prefers a Spring Boot Kotlin style hexagonal layout.

- `domain`
- `application/service`
- `application/port`
- `adapter/inbound`
- `adapter/outbound`
