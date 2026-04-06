# TUI Shell Flow

## Entry Screens
The app still starts on `Home`, then moves to `SessionList`, then to `ConversationShell`. This is useful for diagnostics and browsing, but it is also the main reason the UX still feels like a TUI app with pages instead of one continuous terminal shell.

## Current Screen Responsibilities
- `Home`: startup diagnostics, account state, workspace path, and schema snapshot
- `SessionList`: recent threads, selected-session metadata, and navigation into live shell
- `ConversationShell`: transcript, activity panel, input area, and shell key hints

## Current Conversation Flow
1. Startup checks run in a background thread.
2. Session list loads through `thread/list`.
3. Opening a session loads the snapshot with `thread/read`.
4. Sending a prompt starts a background stream worker.
5. Stream events are sent back into the UI via typed background messages.
6. The shell updates transcript, activity, status, and auto follow-up state.

## Important UI State
The shell already tracks more than page-level loading:

- current thread id and title
- transcript messages
- input buffer
- turn state: draft, ready, submitting, streaming
- auto follow-up state and selected template
- per-turn file-change counts
- warnings and status text

## Current UX Gap
The shell itself is live, but the app frame around it is still navigation-heavy. The next UX step should be reducing screen switching, not rebuilding transcript streaming from scratch.

