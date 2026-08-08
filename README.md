# Akra

[한국어](docs/ko/README.md)

Akra is a native Rust operator client for `codex app-server`. The repository remains named
`codex-exec-loop`, the crate and legacy binary remain `codex-exec-loop-native`, and the operator
command is `akra`.

Akra keeps startup checks, session resume, prompt streaming, accepted planning, queue-driven
continuation, and parallel worker delivery in one long-lived terminal workflow. The TUI is the
primary surface; CLI, Admin, Telegram, and automation adapters reuse the same application services.

## Shipped Surfaces

| Surface | Current role |
| --- | --- |
| Native TUI | Ratatui/Crossterm alternate-screen fullscreen shell with an app-owned transcript viewport, sessions, planning, queue, review, activity, and parallel inspection |
| Core runtime | Headless command/effect/completion/snapshot flow for startup, sessions, turns, streams, and post-turn evaluation |
| Planning | SQLite authority with staged planning artifacts under `.codex-exec-loop/planning/` |
| Parallel mode | Three guarded worktree slots, worker leases, official completion refresh, reviewed GitHub delivery, integration, and cleanup |
| CLI and automation | Planning health, queue/reset operations, structured planning mutation, and manual parallel distributor ticks |
| Admin and Telegram | Loopback Admin UI/API and allowlisted Telegram control plane over the same application services |
| Distribution | Native archives, npm platform packages, validation capture helpers, and tag-driven release workflows |

The exact operator contract is in [Current Product](docs/reference/current-product.md). Architecture
and state ownership are in [Runtime Architecture](docs/reference/architecture.md).

## Install

Prerequisites:

- an official Codex CLI available from a trusted absolute `PATH`
- `codex login` completed
- access to the target workspace
- on Linux, a working Codex sandbox helper (`bwrap` from the Codex installation or the system)

Install the npm package:

```bash
npm install -g @refinedstone/akra
cd /path/to/workspace
akra
```

Build from source:

```bash
. "$HOME/.cargo/env"
cargo build --release
cargo run
```

The native packaging and release contract is in
[Native Packaging and Release](docs/plan/13-native-packaging-and-operator-runbook.md).

## Operator Quick Start

Start `akra` from the repository you want to operate. Startup diagnostics run immediately; input
may be drafted while they finish, but submission waits for readiness.

Common shell commands:

| Command | Purpose |
| --- | --- |
| `:sessions` | search and resume Codex sessions |
| `:queue` / `:q` | inspect accepted and proposed work |
| `:planning` | stage or reopen planning authoring |
| `:directions` | maintain planning directions and idle policy |
| `:reviews` | inspect the review center |
| `:activity [diff\|output]` | inspect retained typed activity for the active turn |
| `:parallel` / `:pa` | enable or refresh the parallel board |
| `:parallel off` | stop local parallel automation without deleting worktrees |
| `:turns <positive\|infinite\|off>` | control single-session auto-follow |
| `:model`, `:think ...`, `:view`, `:language` | choose model, reasoning, transcript detail, and TUI language |
| `:doctor`, `:diag`, `:help` | inspect planning health, startup diagnostics, and command help |

Run `:help` for the complete in-shell registry. See
[Current Product](docs/reference/current-product.md) for keys, planning rules, recovery behavior,
and parallel delivery invariants.

## CLI

```text
akra doctor [workspace_dir]
akra status [workspace_dir]
akra queue [workspace_dir]
akra reset <queue|directions|all> [workspace_dir]
akra planning-tool <contract|run> [workspace_dir]
akra parallel-tick [workspace_dir]
akra admin [--port <port>]
akra telegram [options]
```

`planning-tool run` reads one JSON request from stdin. `admin` binds only to `127.0.0.1` and uses a
capability-backed login/CSRF boundary. Telegram requires a bot token and explicit chat allowlists;
group commands also require an allowed sender.

## State and Configuration

- Planning authority is stored below `${AKRA_HOME:-~/.akra}/projects/<project>/runtime/`.
- Git repositories are bound to an owner-private incarnation marker in the canonical Git common
  directory, so linked worktrees share authority while fresh clones remain isolated.
- `.codex-exec-loop/planning/` contains operator-authored details, prompts, staged drafts, and
  rejected-write evidence; it is not the task-authority database.
- `AKRA_APP_SERVER_PROMPT_LOG=1` opts into bounded prompt/response diagnostics. Trace JSONL is a
  separate opt-in and remains body-redacted.
- GitHub writes require the configured login to match the API credential and repository target;
  verify with `bash scripts/gh-akra.sh auth write-status`.

Security and persistence details belong in
[Runtime Architecture](docs/reference/architecture.md), not in duplicated command documentation.

## Development

```bash
. "$HOME/.cargo/env"
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets --all-features -- -D warnings
```

For broad native/TUI work, run:

```bash
bash scripts/check_native_pr.sh
```

Repository structure, architecture gates, worktree rules, and the GitHub delivery sequence are in
[Development Guide](docs/reference/development.md). Gemini-specific guidance delegates to the same
contract in [.gemini/styleguide.md](.gemini/styleguide.md).

## Documentation

- [Documentation map](docs/README.md)
- [Current product and operator contract](docs/reference/current-product.md)
- [Runtime architecture](docs/reference/architecture.md)
- [Development and delivery guide](docs/reference/development.md)
- [TUI visual contract](docs/design/07-tui-layered-architecture-and-aesthetic-contract.md)
- [Validation records and gates](docs/validation/README.md)
- [Competitive research](docs/competitive/README.md) ([한국어](docs/ko/competitive/README.md))

Current implementation references are compact and have Korean translations under `docs/ko/`.
Future plans are explicitly labeled, and competitive research keeps one current pinned brief per
product plus a compact Korean index.
