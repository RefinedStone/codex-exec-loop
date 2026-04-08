# Hexagonal Runtime Architecture

This is one of the main documents that should retain stable context for phase 2.

## Stable Layer Ownership
- `adapter/inbound/tui`: input mapping, shell state mutation, render preparation, background message handling
- `application/service`: startup, session, conversation, and follow-up orchestration
- `application/port/outbound`: app-server and filesystem boundaries owned by the application layer
- `adapter/outbound`: concrete transport, protocol mapping, retries, warnings, and filesystem access
- `domain`: UI-neutral models for startup diagnostics, sessions, conversations, and follow-up templates

Dependency flow still points inward: `adapter -> application -> domain`.

## Current Runtime Lifecycle
The outbound app-server adapter already behaves like a shared runtime boundary:

- startup checks, session listing, snapshot loading, and turn execution all enter through the same adapter and can reuse one initialized app-server connection
- reconnects and reset-after-failure behavior are surfaced back to the shell as warnings
- the shared runtime is held for the duration of a streaming turn, and request-style actions fall back to an isolated connection only while that shared path is busy
- streamed protocol items are translated into domain-level conversation events before the TUI sees them

## Why This Boundary Matters
- protocol parsing and item mapping are already in the right place
- the application layer can evolve orchestration without pulling transport detail into the UI
- the inbound adapter can change shell structure without forcing domain or outbound rewrites

## Preserve During Phase 2
- keep protocol and filesystem concerns inside outbound adapters
- keep use-case orchestration in application services
- keep domain models free of Ratatui, Crossterm, JSON, and process details
- treat runtime evolution as a lifecycle improvement inside the current architecture, not as a reason to collapse boundaries
