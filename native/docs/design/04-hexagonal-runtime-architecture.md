# Hexagonal Runtime Architecture

## Current Layer Split
The code still follows the intended hexagonal structure:

- `adapter/inbound/tui`: input handling, screen routing, rendering, and shell state mutation
- `application/service`: startup, session, conversation, and follow-up template orchestration
- `application/port/outbound`: app-server and filesystem boundaries
- `adapter/outbound`: concrete app-server and filesystem implementations
- `domain`: shell-neutral models for sessions, startup diagnostics, conversations, and follow-up templates

## Current Runtime Shape
The branch already has a live shell, and the outbound adapter now keeps a shared initialized runtime across both request and turn paths:

- startup checks, recent-session loads, and thread snapshot reads can reuse one initialized app-server connection inside the adapter
- turn execution also reuses that initialized connection, so new-thread and resume-turn flows no longer spawn a fresh app-server process every time
- request-side reconnects and reset-after-failure retries are surfaced back to the shell as warnings instead of staying implicit inside the adapter
- request-style actions fall back to an isolated connection only when the shared runtime is busy streaming a turn
- stream events are translated into domain-level conversation events before reaching the TUI

## Why This Matters
This is a better lifecycle boundary than the fully action-scoped version, but it is not yet a fully continuous runtime. The shell looks live because streamed events are mapped well, and now the turn transport also stays inside the shared adapter runtime unless a concurrent request needs a fallback path.

## Current Strength
The app-server adapter already owns:

- request/response transport
- item-to-domain message mapping
- file change and command execution summaries
- delta and completion notifications
- protocol-level warning capture

That means future runtime changes should preserve the existing adapter boundary instead of pulling protocol logic up into the TUI.

## Recommended Architectural Direction
Keep the current layer ownership, but evolve the outbound runtime toward a longer-lived session-oriented transport. The main target is not a new architecture; it is a better lifecycle inside the existing architecture.
