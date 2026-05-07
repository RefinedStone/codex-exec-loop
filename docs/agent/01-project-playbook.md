# Project Playbook

## Product

- `codex-exec-loop` is a native-first Rust client built on `codex app-server`.
- The operator command is `akra`; the crate and legacy binary name remain `codex-exec-loop-native`.
- Optimize for the TUI and app-server flow first, while keeping CLI, admin API, Telegram, and automation surfaces on the same application services.

## Module Map

- `src/domain/`: pure models such as session summaries and startup diagnostics
- `src/domain/planning/`: planning workspace, direction, task, queue, and validation models
- `src/domain/parallel_mode/`: supervisor, pool, distributor, runtime event, and readiness models
- `src/application/service/`: use-case orchestration such as `StartupService`, `SessionService`, `ConversationService`, prompt assembly, GitHub review polling, planning, and parallel mode
- `src/application/service/planning/`: planning facade plus `admin`, `authoring`, `composition`, `control`, `repair`, `runtime`, `shared`, `task_mutation`, `task_tool`, and `worker` sub-boundaries
- `src/application/service/parallel_mode/`: supervisor, pool, distributor, session-detail, turn, and orchestration boundaries
- `src/application/port/outbound/`: interfaces owned by application services, including app-server, startup probes, session catalog, planning workspace, planning authority, task repository, planning worker, interactive turn runtime, GitHub automation/review polling, parallel runtime, parallel agent worker, and Telegram bot ports
- `src/adapter/inbound/tui/`: Ratatui/Crossterm inline shell, controller/runtime/presentation split, planning overlays, session browser, follow-up overlay, parallel-mode overlay, theme, and terminal testkit
- `src/adapter/inbound/cli.rs`: non-TUI commands for `doctor`, `reset`, `planning-tool`, `parallel-tick`, `admin`, and `telegram`
- `src/adapter/inbound/admin_api/`: Axum planning/admin web UI and JSON API backed by Askama templates
- `src/adapter/inbound/telegram_bot/`: Telegram control-plane runner and message parser
- `src/adapter/outbound/app_server/`: `codex app-server` process/runtime, protocol parsing, execution policy, and planning worker adapters
- `src/adapter/outbound/db/`: SQLite planning authority, task repository, active document, runtime event, lease, session detail, and repo-scoped workspace persistence
- `src/adapter/outbound/filesystem/`: planning workspace file adapter and scaffold/repair support
- `src/adapter/outbound/git/`: local git/worktree runtime operations for parallel mode
- `src/adapter/outbound/github/`: GitHub PR, review, and automation boundary
- `src/adapter/outbound/telegram/`: Telegram HTTP API adapter
- `schema/`: checked-in protocol snapshot used to pin app-server shapes
- `templates/admin/` and `assets/admin/`: admin UI templates and packaged visual assets
- `npm/`: platform wrapper, staged npm package metadata, and runtime tests
- `scripts/`: PR gates, native release packaging, validation capture, planning-tool wrapper, GitHub identity wrapper, and merged-worktree cleanup
- `docs/`: current contracts, design notes, plan history, validation records, release notes, and training references

Keep mapping logic in adapters, not domain models.

## Architecture

- Dependency flow points inward: `adapter -> application -> domain`.
- Inbound adapters translate user events into service calls.
- Application services orchestrate use cases and depend on ports defined in `src/application/port/`.
- Planning changes should enter through `src/application/service/planning/` and the inbound surface that owns the workflow: TUI planning overlays, CLI reports/tools, admin API/forms, or Telegram messages.
- Outbound adapters implement those ports and own process, stdio, JSON, and filesystem details.
- `domain/` stays free of TUI types, transport formats, and external I/O.
- Add a port before a new outbound capability when it improves a real boundary.
- The SQLite authority store is the runtime source for planning, queue, parallel-mode leases, session detail, and distributor event history; filesystem planning files remain the operator-editable mirror and scaffold.
- Parallel mode routes from accepted planning tasks through pool allocation, agent session detail, official completion refresh, GitHub delivery, rebase merge into `prerelease`, and slot cleanup.

## Commands

- `. "$HOME/.cargo/env" && cargo run`: launch the TUI
- `. "$HOME/.cargo/env" && cargo build`: compile the crate
- `. "$HOME/.cargo/env" && cargo test`: run tests
- `. "$HOME/.cargo/env" && cargo fmt`: format source
- `. "$HOME/.cargo/env" && cargo clippy --all-targets --all-features -D warnings`: run when touching lint-sensitive code
- `bash scripts/check_native_pr.sh`: run the native PR gate used by GitHub Actions
- `. "$HOME/.cargo/env" && cargo run -- doctor [workspace_dir]`: inspect planning state from the CLI
- `. "$HOME/.cargo/env" && cargo run -- reset <queue|directions|all> [workspace_dir]`: reset planning state with shared reset rules
- `. "$HOME/.cargo/env" && cargo run -- planning-tool contract`: print the compact JSON contract for automation and LLM tool callers
- `. "$HOME/.cargo/env" && cargo run -- planning-tool run [workspace_dir]`: run a structured planning task request from stdin
- `. "$HOME/.cargo/env" && cargo run -- parallel-tick [workspace_dir]`: manually drive the parallel-mode distributor queue
- `. "$HOME/.cargo/env" && cargo run -- admin [--port <port>]`: run the planning/admin web surface
- `. "$HOME/.cargo/env" && cargo run -- telegram [--token <token>] [--allow-chat-id <chat_id>]...`: run the Telegram control plane

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
- Prefer infra-skippable layout. Storage, GitHub, filesystem, and app-server implementations should sit in boundary-specific directories so a product-flow edit does not have to open them first.
- Keep composition root wiring near entrypoints. Do not make feature logic import concrete leaf adapters just because they are nearby.
- Do not use `use super::*;` or `use super::super::*;` in app runtime, controller, presentation, or planning modules. Import only the symbols the file actually uses.
- Do not rely on parent-module wildcard imports for child modules. Child modules should import their own dependencies explicitly so they remain readable in isolation.
- Keep rendering, presentation projection/builders, controller/event handling, and long integration-style tests in separate files when practical.
- For native shell visual work, follow `docs/design/07-tui-layered-architecture-and-aesthetic-contract.md` before editing: state, controller, projection/copy, theme/chrome, rendering/layout, and terminal adapter concerns must stay separate.
- Split broad shell tests by concern such as planning, session browser, follow-up overlay, or shell surface behavior instead of growing one monolithic test file.
- Treat mixed-responsibility files as design debt even when they still compile cleanly. If one file owns storage writes, recovery, status wording, and operator policy together, split it before adding more behavior.

## Testing

- Place unit tests next to the module with `#[cfg(test)] mod tests`.
- Add integration tests under `tests/` when a flow spans multiple layers.
- Prioritize startup checks, app-server response parsing, stream reduction, and session list mapping.
- New adapter or service logic should usually ship with tests.

## Working Rules

- Use official Codex interfaces first: `codex app-server`, then `codex exec` or `codex exec resume` only when the task still requires them.
- Keep commits small and milestone-based.
- Keep TUI PRs to one primary contract at a time: reducer/runtime, terminal primitive, frame scheduling, or visual snapshot.
- Keep Akra TUI styling behind `AkraTheme`; run `bash scripts/check_tui_layering.sh` when touching native shell visual or presentation code.
- Use `bash scripts/check_native_pr.sh` before opening broad native/TUI PRs; it runs TUI layering, rustfmt check, tests, and clippy in the same order as CI.
- If a TUI bug is being fixed, start by adding the reproducer test or validation capture in the same PR.
- For TUI changes, include a screenshot or short terminal capture in the PR when practical.
- For terminal-affecting TUI PRs, run the focused cargo commands from `docs/plan/29-terminal-ui-testing-methodology.md` and record at least `Windows Terminal / PowerShell / inline` plus `Windows Terminal / WSL bash / inline` with `scripts/capture_native_validation.sh` or `.ps1`.
- Keep backlog, rollout notes, and long-lived design detail in `docs/`, not in agent files.
