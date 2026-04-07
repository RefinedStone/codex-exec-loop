# TUI Shell Flow

## Entry Flow
The app now starts directly in `ConversationShell` with a new-thread draft. Startup diagnostics and recent sessions are still available, but they open as shell overlays instead of replacing the whole screen.

## Current Screen Responsibilities
- `ConversationShell`: single-column transcript, slim status footer, bottom composer, shell key hints, startup/session summaries, and lightweight transcript navigation
- startup overlay: account checks, workspace path, schema snapshot, and warnings
- recent sessions overlay: thread list, selected-session metadata, and resume entry point

## Current Conversation Flow
1. Startup checks run in a background thread.
2. The shell opens immediately in draft mode using the current workspace path.
3. Once startup checks pass, recent sessions load through `thread/list`.
4. Opening a recent session overlay entry loads the snapshot with `thread/read`.
5. Sending a prompt starts a background stream worker.
6. Stream events are sent back into the UI via typed background messages.
7. The shell updates transcript, footer status, composer state, and auto follow-up state.

## Important UI State
The shell already tracks more than page-level loading:

- current thread id and title
- transcript messages
- transcript viewport state for tail-follow versus manual scroll
- input buffer
- overlay state for startup diagnostics and recent sessions
- startup readiness state for gating prompt submission
- turn state: draft, ready, submitting, streaming
- auto follow-up state and selected template
- per-turn file-change counts
- warnings and status text

## Current UX Gap
The shell itself is now the primary frame and the transcript is no longer split by a side activity panel. The default run stays on the main terminal screen, but the app still redraws a full Ratatui viewport in raw mode and still leans on modal overlays, so it has not yet reached a truly append-only scrollback-native CLI feel.

## Next UX Direction
The next UX step is not "more panels" or "more routes". It is a pivot toward a stream-first shell closer to Codex CLI:

- the transcript should feel like one vertical flow rather than a dashboard
- the prompt composer should stay anchored at the bottom
- previous conversation should be readable through terminal scrollback first, with lightweight in-app scrolling only where it still adds value
- diagnostics, session browse, and template inspection should become secondary surfaces instead of competing primary panes
