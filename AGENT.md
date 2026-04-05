# AGENT.md

## Project

`codex-exec-loop` has two tracks.

- Python CLI: wraps `codex exec` and `codex exec resume` for session-based automation.
- Rust native client: a TUI prototype that talks to `codex app-server`.

The current product direction is the Rust client. The goal is a cross-platform Codex-style CLI that feels interactive while staying on top of official Codex surfaces.

## Architecture

Prefer Spring Boot Kotlin style port-and-adapter hexagonal architecture.

- `domain`
  - pure models and business-friendly data types
  - no dependency on adapters
- `application/service`
  - use-case orchestration
  - contains service structs such as `StartupService`, `SessionService`
- `application/port`
  - interfaces owned by the application layer
  - outbound integrations are defined here first
- `adapter/inbound`
  - TUI, CLI, or future API entry points
- `adapter/outbound`
  - Codex app-server integration, filesystem, and other external systems

## Rust Code Style

- Write code so a Spring Boot Kotlin developer can read it quickly.
- Prefer explicit names over compact or clever Rust patterns.
- Keep functions small and single-purpose.
- Use `Service`, `Port`, `Adapter`, `Request`, `Response`, `State` naming consistently.
- Prefer straightforward structs and methods over macro-heavy abstractions.
- Use `Result` for failures at boundaries and avoid `panic!`.
- Keep mapping logic in adapters, not in domain models.
- Keep UI event handling readable even if it is a bit verbose.

## Working Rules

- Use official Codex interfaces first.
  - `codex app-server`
  - `codex exec`
  - `codex exec resume`
- Keep commits small and milestone-based.
- Verify with `cargo fmt`, `cargo build`, and `cargo test` for native changes.
- Do not introduce unnecessary traits. Add a port trait when it improves a boundary.

## Native TODO

- Add real session resume and `thread/start` flow from the selected session.
- Add input box and `turn/start` request handling.
- Render streamed notifications, activity, and approval states in the shell.
- Add session search, paging, and recent filter options.
- Add `Ctrl+C` back navigation.
  - If already on home, pressing it once more should ask for `y/n` before exit.
- Add Windows-focused validation and packaging.
