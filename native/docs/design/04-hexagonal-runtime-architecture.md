# Hexagonal Runtime Architecture

## Current Layer Split
The code still follows the intended hexagonal structure:

- `adapter/inbound/tui`: input handling, screen routing, rendering, and shell state mutation
- `application/service`: startup, session, conversation, and follow-up template orchestration
- `application/port/outbound`: app-server and filesystem boundaries
- `adapter/outbound`: concrete app-server and filesystem implementations
- `domain`: shell-neutral models for sessions, startup diagnostics, conversations, and follow-up templates

## Current Runtime Shape
The branch already has a live shell, but the runtime is still action-scoped:

- snapshot actions open an app-server process, initialize, request data, then exit
- turn execution opens an app-server process, initializes, starts a turn, streams events, then exits
- stream events are translated into domain-level conversation events before reaching the TUI

## Why This Matters
This is a good boundary split, but it is not yet a continuous runtime. The shell looks live because streamed events are mapped well, not because the transport lifecycle is persistent.

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

