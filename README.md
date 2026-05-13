# Akra

Akra is the native Rust terminal client in this repository for `codex app-server`.
The repository name remains `codex-exec-loop`; the operator command is `akra`.

Akra is built for long-lived solo work in one inline shell: startup diagnostics, session resume,
conversation streaming, accepted planning, queue state, post-turn continuation, and parallel-mode
supervision all stay in the terminal. CLI, admin API, Telegram, and automation entrypoints share the
same application services instead of carrying separate planning logic.

## Current Implementation

- TUI: Ratatui/Crossterm inline main-buffer shell with host-terminal scrollback as the durable
  transcript surface.
- Runtime: `codex app-server` process/runtime adapter plus protocol fixtures under `schema/`.
- Planning: SQLite-backed authority store, filesystem planning workspace mirrors, staged draft
  promotion, queue inspection, reset, repair, and planning-worker refresh.
- Supersession: parallel-mode supervisor board, fixed local worktree pool, agent session detail,
  official completion refresh, serialized Git/GitHub delivery, and slot cleanup.
- Operator surfaces: TUI, non-TUI CLI commands, Axum/Askama admin UI/API, Telegram control plane,
  and structured planning-tool automation.
- Distribution: native release bundles, npm platform wrapper packages, validation capture helpers,
  and GitHub Actions for native PR and release checks.

## Implementation Map

- `src/adapter/inbound/tui/`: inline shell, overlays, command palette, model/think controls, session
  browser, diagnostics, planning/directions/queue views, follow-up flow, parallel-mode board, theme,
  and rendering tests.
- `src/adapter/inbound/cli.rs`: `doctor`, `status`, `queue`, `reset`, `planning-tool`,
  `parallel-tick`, `admin`, and `telegram` dispatch.
- `src/adapter/inbound/admin_api/`: planning/admin web UI and JSON API. Templates live in
  `templates/admin/`; packaged visual assets live in `assets/admin/`.
- `src/adapter/inbound/telegram_bot/`: Telegram bot config, message parsing, and control-plane
  runner.
- `src/application/service/planning/`: planning facade, admin workflows, authoring, runtime
  validation/intake, repair/reset, task mutation, task tool, and app-server planning worker
  orchestration.
- `src/application/service/parallel_mode/`: pool allocation, supervisor/readiness, session detail,
  distributor, turn orchestration, Git/GitHub delivery coordination, and cleanup.
- `src/application/port/outbound/`: application-owned contracts for app-server, startup probes,
  session catalog, planning workspace/authority/task repository, planning worker, interactive
  runtime, GitHub, git/worktree runtime, parallel workers, and Telegram.
- `src/adapter/outbound/`: concrete app-server, SQLite, filesystem, git, GitHub, and Telegram
  adapters.
- `schema/`: checked-in `codex app-server` protocol snapshot.
- `npm/`, `.github/workflows/`, and `scripts/`: npm launcher, native packaging, CI gates,
  validation capture, release verification, planning-tool wrapper, and worktree cleanup automation.
- `docs/`: compact current contracts, design notes, workflow rules, validation records, and
  training material.

## Install

### npm

```bash
npm install -g @refinedstone/akra
cd /path/to/workspace
akra
```

Supported npm targets:

- Linux `x64`
- macOS Apple Silicon `arm64`
- Windows `x64`

### Source Install

```bash
git clone https://github.com/RefinedStone/codex-exec-loop.git
cd codex-exec-loop
. "$HOME/.cargo/env"
cargo install --path . --bin akra --locked
cd /path/to/workspace
akra
```

### Local Release Build

```bash
git clone https://github.com/RefinedStone/codex-exec-loop.git
cd codex-exec-loop
. "$HOME/.cargo/env"
cargo build --release
cd /path/to/workspace
/path/to/codex-exec-loop/target/release/codex-exec-loop-native
```

## Quick Start

1. Install the Codex CLI and run `codex login`.
2. Move into the workspace you want to operate on.
3. Launch `akra`.
4. Use `Ctrl+o` or `:sessions` to resume an existing thread, or type into a fresh draft.
5. Use `:doctor` or `akra doctor` to inspect planning state.
6. Use `:planning` to create or reopen the planning control center.
7. Use `:queue` or `akra queue` to inspect the current accepted queue and proposals.

## TUI Controls

### Global Keys

| Key | Purpose |
| --- | --- |
| `Enter` | submit the active prompt or confirm the focused action |
| `Ctrl+j` | insert a newline |
| `Ctrl+t` | open a new draft |
| `Ctrl+o` | open recent sessions, or the supersession board while parallel mode owns the surface |
| `Ctrl+d` | open diagnostics |
| `Ctrl+r` | rerun startup diagnostics or refresh parallel readiness in the board |
| `Ctrl+q` | quit the app |

`Ctrl+c` is a back-or-cancel key inside the app, not the primary quit action.

### Shell Commands

| Command | Purpose |
| --- | --- |
| `:diag` | show startup diagnostics |
| `:parallel`, `:pa` | enter or refresh parallel mode |
| `:parallel off`, `:pa off` | disable local parallel mode and close the board |
| `:sessions` | browse recent sessions |
| `:queue`, `:q` | inspect accepted queue work, proposals, and skipped work |
| `:planning`, `:planning-init` | open the planning control center |
| `:planning doctor`, `:doctor` | inspect planning health |
| `:directions` | maintain direction-side planning artifacts |
| `:reset queue` | reset queue-side planning state |
| `:reset directions confirm` | reset direction-side planning state |
| `:reset all confirm` | replace the full active planning scaffold |
| `:turns <number|infinite>` | set the auto follow-up turn budget |
| `:model` | choose model and reasoning effort |
| `:think <none|minimal|low|medium|high|xhigh|default>` | set reasoning effort |
| `:stop` | stop active app-server sessions |
| `:new` | start a new draft |
| `:help` | list shell commands |

## Non-TUI Commands

| Command | Purpose |
| --- | --- |
| `akra doctor [workspace_dir]` | read-only planning inspection |
| `akra status [workspace_dir]` | print the shared planning status reply |
| `akra queue [workspace_dir]` | print the shared queue reply |
| `akra reset <queue|directions|all> [workspace_dir]` | rewrite the selected accepted planning scope |
| `akra planning-tool contract` | print the compact JSON contract for structured planning-tool callers |
| `akra planning-tool run [workspace_dir]` | execute a structured planning-tool request from stdin |
| `akra parallel-tick [workspace_dir]` | manually drive the parallel-mode distributor queue |
| `akra admin [--port <port>]` | run the planning/admin web UI and JSON API |
| `akra telegram [--token <token>] [--allow-chat-id <chat_id>]...` | run the Telegram control-plane runner |

Aliases remain for compatibility where implemented: `admin-server`, `telegram-bot`, and
`planning-task-tool`.

## Planning And Continuation

- Accepted planning authority is SQLite-backed; tracked planning files are review/export/staged-edit
  artifacts, not the runtime source of truth for git-backed workspaces.
- Runtime derives the current queue task and proposed tasks from accepted planning.
- Builtin `next-task` and internal continuation execute only the accepted queue head.
- Queue-idle behavior follows accepted direction authority.
- Admin/API task intake creates one validated accepted task; it does not interrupt an existing
  `in_progress` task.

The current operator contract lives in
[docs/supersession/current-contract.md](docs/supersession/current-contract.md).

## Development

```bash
. "$HOME/.cargo/env"
cargo build
cargo test
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
```

Use `bash scripts/check_native_pr.sh` for the full native PR gate.

## Packaging And Validation

- Packaging runbook: [docs/plan/13-native-packaging-and-operator-runbook.md](docs/plan/13-native-packaging-and-operator-runbook.md)
- Platform validation matrix: [docs/plan/12-platform-validation-matrix.md](docs/plan/12-platform-validation-matrix.md)
- Validation records: [docs/validation/README.md](docs/validation/README.md)
- Terminal UI testing method: [docs/validation/terminal-ui-testing-methodology.md](docs/validation/terminal-ui-testing-methodology.md)

## Diagnostics

- Debug builds write filtered Akra trace JSONL under `.codex-exec-loop/runtime/log/` by default.
- `AKRA_TRACE=0 cargo run`: disable the default debug trace file.
- `AKRA_TRACE=1 cargo run`: enable the concise Akra debug preset.
- `AKRA_TRACE=planning cargo run`: focus on planning, post-turn evaluation, and planning-worker paths.
- `AKRA_TRACE=full cargo run`: enable global trace output and full span lifecycle events.
- `RUST_LOG=codex_exec_loop_native=trace cargo run`: use standard `tracing_subscriber::EnvFilter`
  syntax.
- `RUSTFLAGS="--cfg tokio_unstable" AKRA_TOKIO_CONSOLE=1 cargo run --features tokio-console`:
  add the tokio-console layer.
- `AKRA_TRACE_FILE=/tmp/akra-trace.jsonl`: override the trace JSONL destination.
- `CODEX_EXEC_LOOP_PLANNER_VISIBILITY=debug cargo run`: expose full planner prompt/response details
  in debug-only TUI surfaces.

## Docs

- [docs/README.md](docs/README.md)
- [docs/supersession/current-contract.md](docs/supersession/current-contract.md)
- [docs/design/01-current-product-state.md](docs/design/01-current-product-state.md)
- [docs/design/02-tui-shell-flow.md](docs/design/02-tui-shell-flow.md)
- [docs/design/04-hexagonal-runtime-architecture.md](docs/design/04-hexagonal-runtime-architecture.md)
- [docs/design/06-planning-runtime-and-draft-editor.md](docs/design/06-planning-runtime-and-draft-editor.md)
