# Akra

Akra is the native Rust operator client for `codex app-server`.

The repository is still named `codex-exec-loop`, the Rust crate and native binary are still
`codex-exec-loop-native`, and the operator command is `akra`.

Akra is built for long-lived work in one terminal session. It keeps startup diagnostics, session
resume, prompt streaming, accepted planning, queue-driven continuation, and parallel worker
supervision in a single inline shell. The TUI is the primary product surface, but CLI commands,
the local admin API, Telegram control, structured planning tools, and release automation all call
the same application services instead of carrying separate planning logic.

## What Is Implemented Now

| Area | Current implementation |
| --- | --- |
| Native shell | Ratatui/Crossterm inline main-buffer TUI. Completed assistant output is inserted into host terminal scrollback; the live viewport keeps the active prompt, stream tail, overlays, and compact notices together. |
| Core runtime | A headless `src/core` command/effect/completion/snapshot boundary coordinates startup, sessions, turn submission, stream reduction, completions, and post-turn evaluation. |
| App-server integration | `src/adapter/outbound/app_server/` starts and speaks to `codex app-server`; checked-in protocol fixtures live under `schema/`. |
| Planning authority | Git-backed workspaces use a repo-scoped SQLite planning authority under the user-level `.akra/projects/<repo-hash>/runtime/` store. Planning files under `.codex-exec-loop/planning/` are operator-facing workspace artifacts, drafts, prompts, and mirrors, not the runtime source of task truth. |
| Queue and continuation | Accepted planning decides the executable queue head, proposed work, skip framing, queue-idle behavior, internal `next-task`, and post-turn continuation. |
| Parallel mode | Git-backed supersession mode manages a fixed local `akra` worktree pool, worker leases, session detail, official completion refresh, serialized distributor delivery, GitHub PR automation, rebase integration into `prerelease`, and slot cleanup. |
| Operator surfaces | Native TUI, non-TUI CLI, local Axum/Askama admin UI and JSON API, Telegram bot runner, and JSON planning-tool automation. |
| Distribution | Native release archives, npm split packages, platform bundles, validation capture scripts, native PR checks, and tag-triggered GitHub Release/npm workflows. |

Current operator-facing behavior is tracked in
[docs/supersession/current-contract.md](docs/supersession/current-contract.md). Architecture details
are under [docs/design/](docs/design/).

## Install

### Prerequisites

- `codex` CLI is installed and available on `PATH`.
- `codex login` has already completed for the operator account.
- The workspace you run `akra` from is the workspace you want Akra to operate on.
- Git-backed planning and parallel mode expect a normal Git repository.
- Parallel GitHub delivery expects usable Git credentials and `gh auth status` for the intended
  GitHub account. Use `AKRA_GITHUB_LOGIN=<login>` when a specific account must be enforced.

### npm

```bash
npm install -g @refinedstone/akra
cd /path/to/workspace
akra
```

The npm package uses a small JavaScript launcher plus platform-specific native optional
dependencies. Supported npm targets are:

- Linux `x64`
- macOS Apple Silicon `arm64`
- Windows `x64`

Node.js `>=18` is required for the npm launcher. The TUI itself runs in the native Rust binary.

### Source

```bash
git clone https://github.com/RefinedStone/codex-exec-loop.git
cd codex-exec-loop
. "$HOME/.cargo/env"
cargo install --path . --bin akra --locked

cd /path/to/workspace
akra
```

### Local Build

```bash
git clone https://github.com/RefinedStone/codex-exec-loop.git
cd codex-exec-loop
. "$HOME/.cargo/env"
cargo build --release

cd /path/to/workspace
/path/to/codex-exec-loop/target/release/codex-exec-loop-native
```

### Native Release Bundle

```bash
cd /path/to/codex-exec-loop
./scripts/package_native_release.sh
```

The bundle contains the native binary, an `akra` launcher or `akra.cmd`, runtime app-server skill
assets, `scripts/gh-akra.sh`, `README.md`, `OPERATOR.md`, examples, `VERSION.txt`, and checksum
files. See [docs/plan/13-native-packaging-and-operator-runbook.md](docs/plan/13-native-packaging-and-operator-runbook.md).

## Quick Start

```bash
cd /path/to/workspace
akra
```

1. Let startup diagnostics finish. The shell can render immediately, but prompt submission is gated
   until diagnostics allow it.
2. Type a prompt and press `Enter`, or use `Ctrl+o` or `:sessions` to resume a prior Codex thread.
3. Use `Ctrl+d` or `:diag` when startup, sessions, app-server, or planning readiness is blocked.
4. Use `:planning` to create or reopen the planning control center.
5. Use `:queue` to inspect accepted queue work, proposals, and skipped work.
6. Use `:parallel` or `:pa` in a git-backed workspace when accepted queue work should run through
   the local parallel worker pool.

The normal loop is one shell: draft, submit, watch the live stream, inspect diagnostics or planning,
resume sessions, let post-turn continuation decide whether queue work advances, and optionally
supervise parallel workers without leaving the terminal.

## Native TUI

### Global Keys

| Key | Purpose |
| --- | --- |
| `Enter` | Submit the active prompt, execute a typed `:` command, or confirm the focused action. |
| `Ctrl+j` | Insert a newline in the active prompt. |
| `Ctrl+t` | Start a blank draft. |
| `Ctrl+o` | Open recent sessions, or close/open the supersession board while parallel mode owns the surface. |
| `Ctrl+d` | Open diagnostics. |
| `Ctrl+r` | Rerun startup diagnostics or refresh parallel readiness in the board. |
| `Ctrl+q` | Quit Akra. |
| `Ctrl+c` | Back or cancel inside the app. It is not the primary quit action. |

### Shell Commands

Typed shell commands begin with `:`. A bare `:` opens the command palette; partial command names
filter suggestions.

| Command | Purpose |
| --- | --- |
| `:diag`, `:diagnostics` | Open startup diagnostics. |
| `:sessions`, `:session` | Browse and resume recent sessions. |
| `:queue`, `:q` | Inspect accepted queue work, proposals, and skip framing. |
| `:planning`, `:planning-init` | Open the planning control center. |
| `:planning doctor`, `:doctor` | Inspect planning health without authoring. |
| `:directions` | Maintain direction-side planning artifacts and queue-idle prompt support. |
| `:reset queue` | Reset queue-side planning state immediately. |
| `:reset directions` | Show reset guidance for direction-side planning state. |
| `:reset directions confirm` | Confirm direction-side reset. |
| `:reset all` | Show reset guidance for the full planning scaffold. |
| `:reset all confirm` | Confirm full planning reset. |
| `:turns <number|infinite>` | Set the internal auto-follow turn budget. |
| `:model` | Open model and reasoning-effort selection. |
| `:model default` | Reset model selection to app-server defaults. |
| `:think <none|minimal|low|medium|high|xhigh|default>` | Set reasoning effort directly. |
| `:view [simple|medium|detail]` | Choose transcript visibility for tool and status rows. |
| `:language [english|korean]`, `:lang [english|korean]` | Choose TUI language. |
| `:stop` | Stop active app-server sessions. |
| `:new` | Start a new draft. |
| `:parallel`, `:pa` | Enable or refresh parallel mode and open the supervisor board. |
| `:parallel off`, `:pa off` | Disable local parallel mode and close the automation epoch. Worktrees remain in place. |
| `:peek` | Inspect active parallel agents. |
| `:help` | Show shell command help. |

### Shell Modes

| Mode | Entry | What it owns |
| --- | --- | --- |
| Conversation | default | Prompt editing, live stream tail, notices, completed transcript insertion into host scrollback. |
| Diagnostics | `Ctrl+d`, `:diag` | Startup readiness, blocking failures, and next operator action. |
| Sessions | `Ctrl+o`, `:sessions` | Session list and resume selection using the current workspace context. |
| Queue | `:queue` | Current accepted queue head, proposals, skipped work, and continuation framing. |
| Planning | `:planning` | Staged planning authoring, validation, and promotion flow. |
| Directions | `:directions` | Direction docs and queue-idle prompt support. |
| Supersession board | `:parallel`, `:pa` | Parallel readiness, slot pool, roster, selected session detail, distributor head, queue state, and withheld-dispatch reason. |

Approval review state is surfaced when app-server reports it, but interactive approve/deny actions
are not exposed in the current TUI.

## Planning Model

Planning is an authority-backed runtime feature, not a side document editor.

- Accepted planning follows `draft -> validate -> promote`.
- In git-backed workspaces, durable task authority lives in SQLite task tables behind
  `PlanningTaskRepositoryPort`.
- Files under `.codex-exec-loop/planning/` remain operator-authored prompts, direction detail docs,
  staged drafts, rejected runtime writes, and review/export artifacts.
- Builtin `next-task` and internal post-turn continuation execute only the accepted queue head.
- Proposed tasks are visible but not executable until promoted or otherwise moved into accepted
  queue state.
- Queue-idle behavior follows accepted direction authority.
- Admin/API task intake creates one validated accepted task and never interrupts an existing
  `in_progress` task.
- Hidden planning workers may refresh queue state through the planning worker boundary, but they do
  not write SQL or tracked planning files directly.

The technical deep dive is
[docs/design/06-planning-runtime-and-draft-editor.md](docs/design/06-planning-runtime-and-draft-editor.md).

## Parallel Mode

Parallel mode, also called supersession in the docs, is the shipped automation path for git-backed
workspaces.

The operator enters it with `:parallel` or `:pa`. The first off-to-on entry checks readiness, opens
the board, resets disposable `akra` pool slots to the current `prerelease` baseline, opens an
automation epoch, and dispatches already-ready accepted queue work up to idle slot capacity.

Important current rules:

- `:parallel on` is not implemented. Use bare `:parallel` or `:pa`.
- Re-running `:parallel` while already enabled refreshes readiness and board projection; it does not
  reset the pool or launch a new epoch by itself.
- `Esc`, `Ctrl+c`, and `Ctrl+o` close the board surface without disabling parallel mode.
- `:parallel off` or `:pa off` disables local parallel mode and clears epoch-local dispatch state,
  but leaves worktrees in place.
- Queue work leases one of three local `akra` slots.
- Worker completion becomes distributor-eligible only after hidden official planning refresh marks
  it commit-ready.
- Distributor delivery is serial: source branch push, PR automation, integration into `prerelease`,
  and slot cleanup.
- Recovery is store-backed. Retryable distributor push recovery is limited to source branch push
  failures; integration branch push blocks remain operator-owned.

Parallel control-plane architecture is documented in
[docs/design/05-parallel-control-plane-architecture.md](docs/design/05-parallel-control-plane-architecture.md).
The board shape is documented in
[docs/design/08-parallel-mode-supersession-board.md](docs/design/08-parallel-mode-supersession-board.md).

## CLI Commands

Running `akra` with no subcommand starts the TUI. The non-TUI commands are operational entrypoints
over the same planning and parallel services.

| Command | Purpose |
| --- | --- |
| `akra doctor [workspace_dir]` | Read-only planning inspection. |
| `akra status [workspace_dir]` | Print the shared planning status reply. |
| `akra queue [workspace_dir]` | Print the shared queue reply. |
| `akra reset <queue|directions|all> [workspace_dir]` | Rewrite the selected accepted planning scope. |
| `akra planning-tool contract` | Print the compact JSON contract for structured planning-tool callers. |
| `akra planning-tool run [workspace_dir]` | Execute a structured planning-tool request from stdin. |
| `akra parallel-tick [workspace_dir]` | Manually drive the parallel-mode distributor queue. |
| `akra admin [--port <port>]` | Run the local planning/admin web UI and JSON API. |
| `akra telegram [--token <token>] [--allow-chat-id <chat_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending]` | Run the Telegram control-plane runner. |

Compatibility aliases remain where implemented:

- `akra admin-server [--port <port>]`
- `akra telegram-bot [--token <token>] [--allow-chat-id <chat_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending]`
- `akra planning-task-tool <contract|run> [workspace_dir]`

## Admin UI And API

`akra admin` starts a loopback-only Axum server for the current workspace.

```bash
cd /path/to/workspace
akra admin --port 18442
```

The default port is `18442`. The server binds to `127.0.0.1` and prints the local URL on startup.

Implemented admin routes include:

- HTML pages under `/admin`, `/admin/directions`, `/admin/tasks`, `/admin/controls`,
  `/admin/drafts`, `/admin/app-server-prompts`, and `/admin/akra/*`.
- JSON planning endpoints under `/api/planning/*`.
- JSON Akra dashboard endpoints under `/api/admin/akra/*`.
- Packaged graphic/game assets under `/admin/assets/*`.

HTML forms use a cookie-backed CSRF token. JSON mutations use the same cookie token mirrored through
the `x-csrf-token` header.

Useful admin environment variables:

- `AKRA_ADMIN_GRAPHIC_ENABLED=0` disables the graphical admin dashboard layer.
- `AKRA_ADMIN_API_BASE_URL=<url>` overrides the API base URL used by the admin graphic client.
- `AKRA_ADMIN_GRAPHIC_POLL_MS=<milliseconds>` sets graphic polling. Values below 5000 are ignored.

## Telegram Control Plane

`akra telegram` runs a local long-polling Telegram bot for the current workspace.

```bash
cd /path/to/workspace
AKRA_TELEGRAM_BOT_TOKEN=<token> \
AKRA_TELEGRAM_ALLOWED_CHAT_IDS=123,456 \
akra telegram
```

Configuration sources are merged in this order:

1. `$XDG_CONFIG_HOME/akra/telegram.env` or `~/.config/akra/telegram.env`
2. Process environment
3. CLI flags

Supported config keys:

```text
AKRA_TELEGRAM_BOT_TOKEN=<token>
AKRA_TELEGRAM_ALLOWED_CHAT_IDS=123,456
```

Supported Telegram commands:

| Command | Purpose |
| --- | --- |
| `/help`, `/start`, `help` | Show help. |
| `/whoami` | Print the current chat id and authorization state. |
| `/status`, `status` | Planning status. |
| `/queue`, `queue` | Planning queue summary. |
| `/plan [status]` | Planning status namespace. |
| `/parallel`, `/parallel status`, `parallel`, `parallel status`, `/parallel_status` | Read-only parallel dashboard status. |
| `/reset queue`, `/reset directions`, `/reset all` | Reset selected planning scope. |
| `/reset_queue`, `/reset_directions`, `/reset_all` | Reset aliases. |

Planning, queue, reset, and parallel status commands require an allowed chat id. `/help` and
`/whoami` remain available so operators can discover the chat id needed for configuration. By
default the runner drops stale Telegram updates on startup; pass `--keep-pending` to process pending
updates instead.

## Runtime Files

Akra reads Codex history and sessions from the normal Codex locations, including
`~/.codex/history.jsonl` and `~/.codex/sessions/`.

For git-backed workspaces, accepted planning authority, runtime projections, leases, distributor
state, and session detail are repo-scoped under the user-level `.akra/projects/<repo-hash>/runtime/`
directory. This lets linked worktrees for the same repository share authority state.

Workspace planning artifacts under `.codex-exec-loop/planning/` are still important, but their role
is operator authoring, staged drafts, prompts, rejected-write inspection, review, and export. Do not
treat tracked planning JSON/files as the authoritative runtime task queue for git-backed workspaces.

## Architecture Map

```text
adapter/inbound/* -> core or application -> domain
core -> application -> domain
application -> outbound ports -> adapter/outbound/*
composition -> concrete wiring
```

Key paths:

| Path | Role |
| --- | --- |
| `src/core/` | Headless app runtime: commands, effects, completions, stream reduction, snapshots. |
| `src/domain/` | Pure models, validation, and invariants. |
| `src/domain/planning/` | Planning workspace, directions, tasks, queue, validation, and projections. |
| `src/domain/parallel_mode/` | Supervisor, pool, distributor, runtime event, readiness, and slot/session rules. |
| `src/application/service/` | Use-case orchestration for startup, sessions, conversations, prompt assembly, planning, post-turn evaluation, GitHub review polling, and parallel mode. |
| `src/application/service/planning/` | Planning facade, admin workflows, authoring, composition, control, repair, runtime intake, shared reports, task mutation, task tool, and planning worker orchestration. |
| `src/application/service/parallel_mode/` | Control-plane, supervisor, pool, distributor, session detail, turn orchestration, Git/GitHub delivery, and cleanup. |
| `src/application/port/outbound/` | Application-owned ports for app-server, startup probes, session catalog, planning authority/workspace/tasks, planning workers, interactive runtime, GitHub, git/worktree runtime, parallel workers, event logs, and Telegram. |
| `src/adapter/inbound/tui/` | Native shell, controllers, overlays, rendering, language/theme controls, terminal adapter, and TUI tests. |
| `src/adapter/inbound/cli.rs` | Non-TUI command dispatch. |
| `src/adapter/inbound/admin_api/` | Admin HTML and JSON API. |
| `src/adapter/inbound/telegram_bot/` | Telegram runner, config, message parser, and control-plane mapping. |
| `src/adapter/outbound/app_server/` | `codex app-server` runtime/process/protocol adapters. |
| `src/adapter/outbound/db/` | SQLite authority, task repository, active documents, runtime events, leases, session detail, distributor queue, and repo-scoped workspace persistence. |
| `src/adapter/outbound/filesystem/` | Planning workspace file adapter and scaffold/repair support. |
| `src/adapter/outbound/git/` | Local git and worktree operations for parallel mode. |
| `src/adapter/outbound/github/` | GitHub PR, review, and automation boundary. |
| `src/adapter/outbound/telegram/` | Telegram HTTP API adapter. |
| `src/composition/` | Production dependency graph wiring. |
| `schema/` | Checked-in app-server protocol snapshot. |
| `templates/admin/`, `assets/admin/` | Admin templates and embedded visual assets. |
| `assets/app-server/skills/` | Runtime app-server skill assets shipped in release bundles. |
| `npm/` | npm launcher, platform resolver, packaging tests, and publish staging. |
| `scripts/` | PR checks, packaging, validation capture, release verification, planning-tool wrapper, GitHub identity wrapper, and worktree cleanup. |
| `docs/` | Current contracts, design references, operational runbooks, validation records, and agent guidance. |

Boundary rules:

- Inbound adapters parse input, render output, and map transport-specific requests.
- `core` owns app lifecycle coordination, not concrete TUI, HTTP, DB, git, or filesystem work.
- Application services own use-case ordering and call ports.
- Domain code owns invariants and pure decisions only.
- Outbound adapters own process, stdio, JSON, SQLite, filesystem, git, GitHub, and Telegram details.
- Mapping logic stays in adapters. Durable policy stays in domain or application services.

See [docs/design/04-hexagonal-runtime-architecture.md](docs/design/04-hexagonal-runtime-architecture.md).

## Development

```bash
. "$HOME/.cargo/env"
cargo build
cargo test
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
```

For the native PR gate used by CI:

```bash
bash scripts/check_native_pr.sh
```

Use focused tests for narrow work:

```bash
cargo test --test architecture_boundaries
cargo test app_server
cargo test planning
```

Preferred coverage areas are startup checks, app-server response parsing, stream reduction, session
list mapping, planning validation, queue projections, admin/API task intake, and parallel
distributor recovery.

## Packaging, Release, And Validation

Build a native archive:

```bash
./scripts/package_native_release.sh --target x86_64-unknown-linux-gnu
```

Verify a bundle and archive:

```bash
./scripts/verify_native_release.sh \
  --archive dist/native/codex-exec-loop-native-<version>-<target>.tar.gz \
  --bundle-dir dist/native/codex-exec-loop-native-<version>-<target>
```

Release tags trigger `.github/workflows/release-native-assets.yml`. The workflow builds Linux,
Windows, and macOS native bundles, verifies them, creates or updates the GitHub Release, and, when
`NPM_TOKEN` is configured, publishes npm platform packages before the main `@refinedstone/akra`
package.

Record terminal validation when a change affects shell rendering, prompt behavior, viewport
handling, scrollback insertion, resize, overlays, status copy, queue/planning surfaces, or
parallel-mode operator flow:

```bash
bash scripts/capture_native_validation.sh \
  --frontend inline \
  --check-profile terminal-baseline \
  --terminal "iTerm2 3.5" \
  --result pass \
  --output-dir docs/validation
```

Summarize recorded validation:

```bash
bash scripts/summarize_native_validation.sh
```

Validation references:

- [docs/plan/12-platform-validation-matrix.md](docs/plan/12-platform-validation-matrix.md)
- [docs/validation/README.md](docs/validation/README.md)
- [docs/validation/terminal-ui-testing-methodology.md](docs/validation/terminal-ui-testing-methodology.md)

## Diagnostics And Tracing

Debug builds write filtered Akra trace JSONL under `.codex-exec-loop/runtime/log/` by default.

| Setting | Effect |
| --- | --- |
| `AKRA_TRACE=0 cargo run` | Disable the default debug trace file. |
| `AKRA_TRACE=1 cargo run` | Enable the concise Akra debug preset. |
| `AKRA_TRACE=planning cargo run` | Focus on planning, post-turn evaluation, and planning-worker paths. |
| `AKRA_TRACE=full cargo run` | Enable global trace output and full span lifecycle events. |
| `RUST_LOG=codex_exec_loop_native=trace cargo run` | Use standard `tracing_subscriber::EnvFilter` syntax. |
| `AKRA_TRACE_FILE=/tmp/akra-trace.jsonl` | Override the trace JSONL destination. |
| `CODEX_EXEC_LOOP_PLANNER_VISIBILITY=debug cargo run` | Expose full planner prompt/response details in debug-only TUI surfaces. |
| `RUSTFLAGS="--cfg tokio_unstable" AKRA_TOKIO_CONSOLE=1 cargo run --features tokio-console` | Add the tokio-console layer. |

Useful GitHub review/polling variables:

- `CODEX_EXEC_LOOP_GITHUB_PR=owner/repo#123`
- `CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS=60`
- `AKRA_GITHUB_LOGIN=<login>`
- `AKRA_GITHUB_TOKEN=<token>`

## Current Limits

- Real-terminal validation is still required after changes to prompt editing, streaming, overlays,
  terminal restore, restart recovery, blocked distributor flow, or multi-worktree operation.
- Planning detail mode supports manual authoring only; `llm-assisted` planning authoring is
  disabled.
- The checked-in app-server schema snapshot still predates newer approval response methods, so the
  TUI does not expose approve or deny actions.
- Non-git workspaces do not use the full supersession worktree pool model.
- Release archive file names use the package version declared in `Cargo.toml`; npm publish staging
  uses the pushed tag version.

## Documentation Index

- [docs/README.md](docs/README.md): compact map of current docs.
- [docs/supersession/current-contract.md](docs/supersession/current-contract.md): shipped planning,
  continuation, and parallel-mode operator contract.
- [docs/design/01-current-product-state.md](docs/design/01-current-product-state.md): product
  identity, surface map, runtime shape, and code entry.
- [docs/design/02-tui-shell-flow.md](docs/design/02-tui-shell-flow.md): operator-visible shell
  modes and flow.
- [docs/design/04-hexagonal-runtime-architecture.md](docs/design/04-hexagonal-runtime-architecture.md):
  layer ownership and architecture gates.
- [docs/design/05-parallel-control-plane-architecture.md](docs/design/05-parallel-control-plane-architecture.md):
  parallel-mode ownership and control-plane rules.
- [docs/design/06-planning-runtime-and-draft-editor.md](docs/design/06-planning-runtime-and-draft-editor.md):
  planning authority, staged drafts, runtime task intake, and recovery.
- [docs/design/08-parallel-mode-supersession-board.md](docs/design/08-parallel-mode-supersession-board.md):
  shipped parallel board shape.
- [docs/plan/13-native-packaging-and-operator-runbook.md](docs/plan/13-native-packaging-and-operator-runbook.md):
  native bundle, npm, release, and operator handoff.
