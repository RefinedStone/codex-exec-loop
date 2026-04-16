# Project Playbook

## Product

- `codex-exec-loop` is a native-first Rust client built on `codex app-server`.
- Optimize for the TUI and app-server flow first.

## Module Map

- `src/domain/`: pure models such as session summaries and startup diagnostics
- `src/application/service/`: use-case orchestration such as `StartupService` and `ConversationService`
- `src/application/service/planning/`: planning feature facades exposed to the TUI
- `src/application/port/`: interfaces owned by the application layer
- `src/adapter/inbound/tui/`: Ratatui/Crossterm screens and event handling
- `src/adapter/outbound/`: `codex app-server` integration and filesystem adapters
- `schema/`: checked-in protocol snapshot used to pin app-server shapes
- `docs/`: design notes, state docs, and validation references

Keep mapping logic in adapters, not domain models.

## Architecture

- Dependency flow points inward: `adapter -> application -> domain`.
- Inbound adapters translate user events into service calls.
- Application services orchestrate use cases and depend on ports defined in `src/application/port/`.
- Planning changes should enter through `src/application/service/planning/` and `src/adapter/inbound/tui/app/planning/` before touching lower-level planning internals.
- Outbound adapters implement those ports and own process, stdio, JSON, and filesystem details.
- `domain/` stays free of TUI types, transport formats, and external I/O.
- Add a port before a new outbound capability when it improves a real boundary.

## Commands

- `. "$HOME/.cargo/env" && cargo run`: launch the TUI
- `. "$HOME/.cargo/env" && cargo build`: compile the crate
- `. "$HOME/.cargo/env" && cargo test`: run tests
- `. "$HOME/.cargo/env" && cargo fmt`: format source
- `. "$HOME/.cargo/env" && cargo clippy --all-targets --all-features -D warnings`: run when touching lint-sensitive code

## Style

- Write Rust so a Spring Boot Kotlin developer can read it quickly.
- Use 4-space indentation, `snake_case` for functions and modules, and `PascalCase` for types.
- Prefer explicit names over clever Rust patterns.
- Keep functions small and single-purpose.
- Use `Service`, `Port`, `Adapter`, `Request`, `Response`, and `State` naming consistently.
- Prefer straightforward structs and methods over macro-heavy abstractions.
- Use `Result` for boundary failures and avoid `panic!`.
- Keep UI event handling readable even when that means a little more code.

## LLM Context Budget

- Treat `src/adapter/inbound/tui/app/` as an LLM-facing surface: reduce the number of files and types needed for a safe edit.
- Keep new TUI/controller/presentation files near 600 LOC or less; once a file crosses roughly 800 LOC, split it by subsystem or view concern in the same PR.
- Do not use `use super::*;` or `use super::super::*;` in app runtime, controller, presentation, or planning modules. Import only the symbols the file actually uses.
- Do not rely on parent-module wildcard imports for child modules. Child modules should import their own dependencies explicitly so they remain readable in isolation.
- Keep rendering, presentation projection/builders, controller/event handling, and long integration-style tests in separate files when practical.
- Split broad shell tests by concern such as planning, session browser, follow-up overlay, or shell surface behavior instead of growing one monolithic test file.

## Testing

- Place unit tests next to the module with `#[cfg(test)] mod tests`.
- Add integration tests under `tests/` when a flow spans multiple layers.
- Prioritize startup checks, app-server response parsing, stream reduction, and session list mapping.
- New adapter or service logic should usually ship with tests.

## Working Rules

- Use official Codex interfaces first: `codex app-server`, then `codex exec` or `codex exec resume` only when the task still requires them.
- Keep commits small and milestone-based.
- For TUI changes, include a screenshot or short terminal capture in the PR when practical.
- Keep backlog, rollout notes, and long-lived design detail in `docs/`, not in agent files.
