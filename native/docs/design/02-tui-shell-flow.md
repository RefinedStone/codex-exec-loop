# TUI Shell Flow

## Entry Flow
The app now starts directly in `ConversationShell` with a new-thread draft. Startup diagnostics and recent sessions are still available, but they open as shell overlays instead of replacing the whole screen.

## Current Screen Responsibilities
- `ConversationShell`: transcript, activity panel, input area, shell key hints, and startup/session summaries
- startup overlay: account checks, workspace path, schema snapshot, and warnings
- recent sessions overlay: thread list, selected-session metadata, and resume entry point

## Current Conversation Flow
1. Startup checks run in a background thread.
2. The shell opens immediately in draft mode using the current workspace path.
3. Once startup checks pass, recent sessions load through `thread/list`.
4. Opening a recent session overlay entry loads the snapshot with `thread/read`.
5. Sending a prompt starts a background stream worker.
6. Stream events are sent back into the UI via typed background messages.
7. The shell updates transcript, activity, status, and auto follow-up state.

## Important UI State
The shell already tracks more than page-level loading:

- current thread id and title
- transcript messages
- input buffer
- overlay state for startup diagnostics and recent sessions
- startup readiness state for gating prompt submission
- turn state: draft, ready, submitting, streaming
- auto follow-up state and selected template
- per-turn file-change counts
- warnings and status text

## Current UX Gap
The shell itself is now the primary frame, but the experience is still not fully continuous. The next UX step should focus on runtime continuity and shell ergonomics, not rebuilding transcript streaming from scratch.
