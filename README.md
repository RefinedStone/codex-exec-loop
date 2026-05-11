# Akra

Akra is the native Rust terminal client in this repository for `codex app-server`.
The repository name remains `codex-exec-loop`; the operator-facing command is `akra`.

It is built for long-lived solo work: startup diagnostics, session resume, accepted planning,
the current queue task, proposed tasks, and internal post-turn continuation all stay inside one inline shell.
The same application layer also drives the non-TUI operator surfaces: planning/admin web UI,
structured planning-tool calls, parallel-mode distributor ticks, and Telegram control-plane commands.

## Why this repo exists

- Inline shell is the only frontend. The host terminal scrollback is the durable transcript view.
- Startup checks run immediately and the shell can accept buffered input before diagnostics fully settle.
- Session resume is a first-class workflow, not an afterthought.
- Planning is part of the main operator loop, with accepted planning shaping the current queue task,
  proposed tasks, and next-task behavior.
- The project ships native packaging, validation capture helpers, and release automation rather than relying on ad hoc local setup.

## Status

- Current supersession, planning, and directions contract: [docs/supersession/current-contract.md](docs/supersession/current-contract.md)
- Remaining supersession and planning follow-through: [docs/supersession/remaining-work.md](docs/supersession/remaining-work.md)
- Agent entrypoint and repository working rules: [AGENTS.md](AGENTS.md)
- Architecture and boundary rules: [docs/design/04-hexagonal-runtime-architecture.md](docs/design/04-hexagonal-runtime-architecture.md)
- Product identity and surface map: [docs/design/01-current-product-state.md](docs/design/01-current-product-state.md)
- TUI shell flow deep dive: [docs/design/02-tui-shell-flow.md](docs/design/02-tui-shell-flow.md)
- Planning/runtime technical deep dive: [docs/design/06-planning-runtime-and-draft-editor.md](docs/design/06-planning-runtime-and-draft-editor.md)
- Active context-first roadmap: [docs/plan/20-context-first-architecture-and-doc-coherence.md](docs/plan/20-context-first-architecture-and-doc-coherence.md)
- Active terminal-agent bridge research hub: [docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md](docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md)
- Docs map and source-of-truth index: [docs/README.md](docs/README.md)
- Supersession docs hub: [docs/supersession/README.md](docs/supersession/README.md)

## Architecture Principles

- Dependency flow stays `adapter -> application -> domain`.
- Operator-visible flows should be readable with a small local context instead of requiring a repo-wide scan.
- Infrastructure details such as DB, GitHub, filesystem, and app-server adapters should live behind clear directory and port boundaries so main product logic can skip them.
- Large files are a boundary smell. Split mixed-responsibility files by subsystem before they become the only safe place to edit.

The design baseline lives in [docs/design/04-hexagonal-runtime-architecture.md](docs/design/04-hexagonal-runtime-architecture.md).
Current structural debt, hotspot order, and refactor targets live in [docs/plan/17-structure-and-architecture-debt-map.md](docs/plan/17-structure-and-architecture-debt-map.md).
The active roadmap for the next cycle lives in [docs/plan/20-context-first-architecture-and-doc-coherence.md](docs/plan/20-context-first-architecture-and-doc-coherence.md) and the terminal-agent bridge research set rooted at [docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md](docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md).

## Implementation Map

- `src/adapter/inbound/tui/`: Ratatui/Crossterm inline shell, command palette, session browser, diagnostics, planning/directions/queue/task overlays, follow-up flow, parallel-mode overlay, and terminal rendering tests.
- `src/adapter/inbound/cli.rs`: process command dispatcher for `doctor`, `reset`, `planning-tool`, `parallel-tick`, `admin`, and `telegram`.
- `src/adapter/inbound/admin_api/`: Axum planning/admin UI plus JSON API. Askama templates live in `templates/admin/`; packaged admin visual assets live in `assets/admin/`.
- `src/adapter/inbound/telegram_bot/`: Telegram bot config, message parsing, and control-plane runner.
- `src/application/service/planning/`: planning facade, admin workflows, authoring, runtime validation/intake, repair/reset, task mutation, task tool, and app-server planning worker orchestration.
- `src/application/service/parallel_mode/`: pool allocation, supervisor/readiness, session detail, distributor, turn orchestration, and GitHub delivery workflow coordination.
- `src/application/port/outbound/`: application-owned contracts for app-server, startup probes, session catalog, planning workspace/authority/task repository, planning worker, interactive runtime, GitHub, git/worktree runtime, parallel workers, and Telegram.
- `src/adapter/outbound/`: concrete app-server, SQLite, filesystem, git, GitHub, and Telegram adapters.
- `schema/`: checked-in `codex app-server` protocol snapshot.
- `npm/`, `.github/workflows/`, and `scripts/`: native packaging, npm platform wrapper, CI gates, validation capture, release verification, planning-tool wrapper, and worktree cleanup automation.

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

### Source install

```bash
git clone https://github.com/RefinedStone/codex-exec-loop.git
cd codex-exec-loop
. "$HOME/.cargo/env"
cargo install --path . --bin akra --locked
cd /path/to/workspace
akra
```

### Local release build

```bash
git clone https://github.com/RefinedStone/codex-exec-loop.git
cd codex-exec-loop
. "$HOME/.cargo/env"
cargo build --release
cd /path/to/workspace
/path/to/codex-exec-loop/target/release/codex-exec-loop-native
```

### Packaged bundle

Git tags publish native bundles through `.github/workflows/release-native-assets.yml`.
If you unpack a release bundle and put it on `PATH`, you can launch `akra` directly from any workspace.

## Quick Start

1. Install the Codex CLI and run `codex login`.
2. Move into the workspace you want to operate on.
3. Launch `akra`.
4. Use `Ctrl+o` or `:sessions` to resume an existing thread, or start typing into a fresh draft.
5. Use `:doctor` to inspect planning state, or `:planning` to create the default planning scaffold.
6. Use `:queue` to inspect the current queue task and proposed tasks, and `:planning` or `:directions` to manage accepted planning.

## Core Surfaces

### Global keys

| Key | Purpose |
| --- | --- |
| `Enter` | submit the active prompt |
| `Ctrl+j` | insert a newline |
| `Ctrl+t` | open a new draft |
| `Ctrl+o` | open recent sessions |
| `Ctrl+d` | open diagnostics |
| `Ctrl+r` | rerun startup diagnostics |
| `Ctrl+q` | quit the app |

`Ctrl+c` is a back-or-cancel key inside the app, not the primary quit action.

### Shell commands

| Command | Purpose |
| --- | --- |
| `:diag` | show startup diagnostics |
| `:parallel [off]` | enter parallel mode or turn it off |
| `:sessions` | browse recent sessions |
| `:queue` | inspect the current queue task, proposed tasks, and skipped work |
| `:planning` | open planning controls |
| `:planning doctor` | inspect planning health from the planning command surface |
| `:directions` | manage direction-side planning artifacts |
| `:task [prompt]` | preview and stage a runtime planning task |
| `:turns <number|infinite>` | set the auto follow-up turn budget |
| `:stop` | stop active app-server sessions |
| `:doctor` | inspect planning health inside the shell |
| `:reset <queue|directions|all>` | reset planning state with explicit target semantics |
| `:new` | start a new draft |
| `:help` | list available shell commands |

Supported aliases remain available for common commands such as `:pa`, `:q`, `:diagnostics`, and `:session`.

### External planning lifecycle commands

| Command | Purpose |
| --- | --- |
| `akra doctor [workspace_dir]` | read-only planning inspection |
| `akra reset <queue|directions|all> [workspace_dir]` | rewrite planning state with shared reset rules |
| `akra planning-tool contract` | print the compact JSON contract for structured planning tool callers |
| `akra planning-tool run [workspace_dir]` | execute a structured planning tool request from stdin |
| `akra parallel-tick [workspace_dir]` | manually drive the parallel-mode distributor queue |
| `akra admin [--port <port>]` | run the planning/admin web UI and JSON API |
| `akra telegram [--token <token>] [--allow-chat-id <chat_id>]...` | run the Telegram control-plane runner |

## Planning And Internal Continuation

Planning and post-turn continuation are organized around accepted planning rather than ad hoc prompt files.

- The operator owns staged drafts and explicit promotion.
- Runtime derives the current queue task and proposed tasks from accepted planning.
- Builtin next-task logic only acts on the current queue task.
- Queue-idle behavior follows the accepted planning policy.

The canonical planning and supersession contract lives in [docs/supersession/current-contract.md](docs/supersession/current-contract.md).

## Packaging And Validation

- Packaging runbook: [docs/plan/13-native-packaging-and-operator-runbook.md](docs/plan/13-native-packaging-and-operator-runbook.md)
- Validation matrix: [docs/plan/12-platform-validation-matrix.md](docs/plan/12-platform-validation-matrix.md)
- Validation records: [docs/validation/README.md](docs/validation/README.md)
- Validation capture helpers:
  - `scripts/capture_native_validation.sh`
  - `scripts/capture_native_validation.ps1`

## Development

```bash
. "$HOME/.cargo/env"
cargo build
cargo test -- --nocapture
cargo fmt --all
cargo clippy --all-targets --all-features -D warnings
```

## Diagnostics

- `cargo run`: in debug Akra binaries, write filtered development trace JSONL to daily rolling files under `.codex-exec-loop/runtime/log/`.
- `tail -f .codex-exec-loop/runtime/log/akra-trace.jsonl* | jq 'select(.event)'`: follow Akra diagnostic events in the trace log.
- `AKRA_TRACE=0 cargo run`: disable the default debug trace file.
- `AKRA_TRACE=1 cargo run`: enable the concise Akra debug preset under `codex_exec_loop_native::diagnostics::akra_event`.
- `AKRA_TRACE=planning cargo run`: focus trace output on planning, post-turn evaluation, and app-server planning-worker paths.
- `AKRA_TRACE=full cargo run`: enable the old noisy behavior with global `trace` and full span lifecycle events.
- `RUST_LOG=codex_exec_loop_native=trace cargo run`: use standard `tracing_subscriber::EnvFilter` syntax for module-level filtering.
- `RUSTFLAGS="--cfg tokio_unstable" AKRA_TOKIO_CONSOLE=1 cargo run --features tokio-console`: add the tokio-console layer while retaining file logging.
- `AKRA_TRACE_SPANS=none|close|full`: override span events for any trace preset or custom filter.
- `AKRA_TRACE=codex_exec_loop_native::application::service::planning=debug cargo run`: trace a selected module filter.
- `AKRA_TRACE_FILE=/tmp/akra-trace.jsonl`: override the trace JSONL destination with an exact append file instead of daily rolling.
- `jq -r 'select(.event=="user_prompt_submit_inspected") | .transcript_text' /tmp/akra-trace.jsonl`: inspect the exact operator prompt submitted through the TUI.
- Parallel sub-session trace events retain task/worktree identifiers and content lengths, but not raw handoff prompts, developer instructions, or assistant reply text.
- `akra_event!(tracing::Level::DEBUG, "message", key = value)`: emit standard structured Akra diagnostics without evaluating field expressions when the target is disabled. The trace formatter flattens the lazy JSON detail into top-level JSONL fields while preserving string, number, and boolean types.
- `codex_exec_loop_native::diagnostics::dropped_log_lines()`: read the in-process trace `tracing_appender` dropped-line counter for future doctor or health surfaces.
- `akra_diagnostics_dropped_log_lines`: emitted as a shutdown warning when the non-blocking trace queue dropped any lines.
- `CODEX_EXEC_LOOP_PLANNER_VISIBILITY=debug cargo run`: expose full planner prompt/response details in debug-only TUI surfaces.

## Docs

- [docs/supersession/current-contract.md](docs/supersession/current-contract.md)
- [docs/supersession/remaining-work.md](docs/supersession/remaining-work.md)
- [docs/design/01-current-product-state.md](docs/design/01-current-product-state.md)
- [docs/design/02-tui-shell-flow.md](docs/design/02-tui-shell-flow.md)
- [docs/design/06-planning-runtime-and-draft-editor.md](docs/design/06-planning-runtime-and-draft-editor.md)
- [docs/README.md](docs/README.md)
