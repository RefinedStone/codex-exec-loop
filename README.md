# Akra

Akra is the native Rust terminal client in this repository for `codex app-server`.
The repository name remains `codex-exec-loop`; the operator-facing command is `akra`.

It is built for long-lived solo work: startup diagnostics, session resume, accepted planning state,
queue inspection, and post-turn automation all stay inside one inline shell.

## Why this repo exists

- Inline shell is the only frontend. The host terminal scrollback is the durable transcript view.
- Startup checks run immediately and the shell can accept buffered input before diagnostics fully settle.
- Session resume is a first-class workflow, not an afterthought.
- Planning is part of the main operator loop, with accepted queue state shaping automation and next-task behavior.
- The project ships native packaging, validation capture helpers, and release automation rather than relying on ad hoc local setup.

## Status

- Current product contract: [docs/design/01-current-product-state.md](docs/design/01-current-product-state.md)
- Planning contract: [docs/design/06-planning-runtime-and-draft-editor.md](docs/design/06-planning-runtime-and-draft-editor.md)
- Release delta from `v1.2.9`: [docs/releases/v1.2.9-to-prerelease.md](docs/releases/v1.2.9-to-prerelease.md)
- Remaining supersession follow-through and open questions: [docs/supersession/README.md](docs/supersession/README.md)

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
5. Use `:doctor` to inspect planning state, or `:init` to create the default planning scaffold.
6. Use `Ctrl+f` or `:auto` to inspect post-turn automation and `:queue` to inspect accepted queue state.

## Core Surfaces

### Global keys

| Key | Purpose |
| --- | --- |
| `Enter` | submit the active prompt |
| `Ctrl+j` | insert a newline |
| `Ctrl+t` | open a new draft |
| `Ctrl+o` | open recent sessions |
| `Ctrl+d` | open diagnostics |
| `Ctrl+f` | open automation controls |
| `Ctrl+r` | rerun startup diagnostics |
| `Ctrl+q` | quit the app |

`Ctrl+c` is a back-or-cancel key inside the app, not the primary quit action.

### Shell commands

| Command | Purpose |
| --- | --- |
| `:diag` | show startup diagnostics |
| `:sessions` | browse recent sessions |
| `:auto` | open automation controls |
| `:queue` | inspect queue head, proposals, and skipped work |
| `:planning` | open planning controls |
| `:directions` | manage direction-side planning artifacts |
| `:doctor` | inspect planning health inside the shell |
| `:init` | open the default planning scaffold review |
| `:reset <queue|directions|all>` | reset planning state with explicit target semantics |
| `:new` | start a new draft |
| `:help` | list available shell commands |

Supported aliases remain available for common commands such as `:q`, `:diagnostics`, `:session`,
`:automation`, `:turn 10`, and `:auto-turns 10`.

### External planning lifecycle commands

| Command | Purpose |
| --- | --- |
| `akra doctor [workspace_dir]` | read-only planning inspection |
| `akra init [workspace_dir]` | create the default simple planning scaffold |
| `akra reset <queue|directions|all> [workspace_dir]` | rewrite planning state with shared reset rules |

## Planning And Automation

Accepted planning state lives under `.codex-exec-loop/planning/`.

- The operator owns staged drafts and explicit promotion.
- Runtime owns queue snapshots and rejected-write archives.
- Builtin next-task logic only acts on accepted queue state.
- Queue-idle behavior is driven by `[queue_idle]` in `directions.toml`.

Full planning behavior lives in [docs/design/06-planning-runtime-and-draft-editor.md](docs/design/06-planning-runtime-and-draft-editor.md).

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

## Docs

- [docs/design/01-current-product-state.md](docs/design/01-current-product-state.md)
- [docs/design/02-tui-shell-flow.md](docs/design/02-tui-shell-flow.md)
- [docs/design/06-planning-runtime-and-draft-editor.md](docs/design/06-planning-runtime-and-draft-editor.md)
- [docs/README.md](docs/README.md)
