# TUI Shell Flow

This file describes operator-visible shell behavior. It does not describe how to implement it.

## Main Flow

- The shell header shows startup, session, and conversation identity.
- The transcript panel renders conversation history plus inline status and runtime notices.
- The composer accepts both free-form prompts and `:` commands in the main buffer.
- The footer surfaces runtime state, follow-up state, planning state, and planner debug detail when enabled.

## Startup And Sessions

- Startup diagnostics begin on launch.
- The shell may render before diagnostics finish, but submit only proceeds when diagnostics allow it.
- Queued manual input auto-submits once startup becomes ready.
- `Ctrl+o` opens recent sessions.
- Selecting a session loads its snapshot into the main shell.
- `Ctrl+n` returns to a blank draft shell.

## Inline Commands

- `:diagnostics` toggles startup diagnostics.
- `:sessions` toggles recent sessions.
- `:templates` opens follow-up controls.
- `:queue` opens planning queue state.
- `:planning` opens planning workspace controls.
- `:planning on|off` toggles planning mode for the current workspace.
- `:directions` opens directions maintenance.
- `:stop` stops post-turn automation.

## Follow-Up And Planning

- Follow-up controls own template selection, stop rules, and planner debug visibility.
- Builtin `next-task` uses accepted planning state only.
- `:planning` and `:directions` both route through the embedded draft editor.
- Planning state is reflected in the footer, follow-up overlay, queue overlay, and post-turn automation.

## Code Entry

- Generic shell state reducers live under `src/adapter/inbound/tui/app`.
- Planning-specific TUI flow lives under `src/adapter/inbound/tui/app/planning`.
