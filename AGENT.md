# AGENT.md

## Project

`codex-exec-loop` is now a native-first project.

- Rust native client: the primary product track, built on `codex app-server`
- Python CLI: legacy compatibility path only, kept temporarily during native migration

The product goal is a cross-platform Codex-style CLI that feels interactive and can continue work automatically with canned follow-up prompts.

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
  - keep `codex exec` / `codex exec resume` only for legacy compatibility work
- Keep commits small and milestone-based.
- After finishing a meaningful feature or refactor:
  - commit the change set
  - push the working branch
  - open a pull request to the intended base branch unless blocked by missing permissions or user instruction
  - after a PR is merged or closed, do not continue on the same feature branch
  - start the next task from the latest target base branch on a new feature branch and open a new PR
- After PR review arrives:
  - inspect every new review comment and thread before changing code
  - fix correctness and low-cost maintainability issues that align with the chosen architecture
  - reply on each review thread with the applied fix or the rationale for not changing direction
  - commit and push the review response separately from the original milestone commit when practical
  - rebase the feature branch onto the latest target base branch before merge
  - merge by updating the base branch fast-forward to the reviewed feature head when possible
- Verify with `cargo fmt`, `cargo build`, and `cargo test` for native changes.
- Do not introduce unnecessary traits. Add a port trait when it improves a boundary.
- Review handling:
  - fix correctness, deadlock, crash, data-loss, and clear state-management issues
  - fix low-cost maintainability improvements when they do not fight the chosen architecture
  - if feedback pushes away from the intended Spring Boot Kotlin style or the chosen hexagonal structure, reply with the rationale and close the thread without changing direction

## Native TODO

- Make native auto-follow-up/template selection the main workflow.
- Render streamed notifications, activity, and approval states in the shell.
- Add session search, paging, and recent filter options.
- Add GitHub PR review/comment change detection.
  - Start with polling.
  - Webhook notification can come later.
- Add Windows-focused validation and packaging.
