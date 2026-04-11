# codex-exec-loop

`codex-exec-loop` is a Rust TUI client built on `codex app-server`.

The repository root is the product root. Run the app, tests, docs, and helper scripts from here.

## Quick Start

Requirements:

- Codex CLI installed
- Codex login completed
- Rust toolchain available

Run the client:

```bash
cd /path/to/codex-exec-loop
. "$HOME/.cargo/env"
cargo run
```

Frontend selection:

- `CODEX_EXEC_LOOP_FRONTEND=inline`: default inline main-buffer shell
- `CODEX_EXEC_LOOP_FRONTEND=alternate`: fullscreen alternate-screen shell
- `CODEX_EXEC_LOOP_ALT_SCREEN=1`: legacy alternate-screen fallback

Optional GitHub polling:

- `CODEX_EXEC_LOOP_GITHUB_PR=owner/repo#123`
- `CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS=60`

## Current Capability

- startup diagnostics and immediate shell entry
- new draft start, existing thread resume, snapshot loading, and streamed turns
- inline inspections for diagnostics, sessions, follow-up templates, and planning
- recent session search, paging, and current-workspace filtering
- builtin and workspace follow-up templates, max-turn control, stop keyword, and no-file-change stop
- planning bootstrap via `:planning`, embedded draft editor, staged draft promote, queue summary, and invalid task-ledger repair retry
- approval, tool activity, runtime warning, and GitHub review-change visibility
- packaging, checksum, and platform-validation helper scripts

## Docs

Start with:

- [docs/README.md](./docs/README.md)
- [docs/design/01-current-product-state.md](./docs/design/01-current-product-state.md)
- [docs/design/06-planning-runtime-and-draft-editor.md](./docs/design/06-planning-runtime-and-draft-editor.md)
- [docs/plan/13-native-packaging-and-operator-runbook.md](./docs/plan/13-native-packaging-and-operator-runbook.md)
- [docs/plan/12-platform-validation-matrix.md](./docs/plan/12-platform-validation-matrix.md)

## Packaging And Validation

Build a release bundle:

```bash
./scripts/package_native_release.sh
```

Verify a generated bundle:

```bash
./scripts/verify_native_release.sh \
  --archive dist/native/codex-exec-loop-native-<version>-<target>.tar.gz \
  --bundle-dir dist/native/codex-exec-loop-native-<version>-<target>
```

Capture a validation record:

```bash
./scripts/capture_native_validation.sh \
  --frontend inline \
  --result pass \
  --output-dir docs/validation
```

Summarize recorded rows:

```bash
./scripts/summarize_native_validation.sh
```

The release bundle carries the checked-in sample prompt assets under `examples/` and `.codex-exec-loop/followups/` in addition to the binary and operator docs.

## Repository Guide

- `src/`: Rust application code
- `schema/`: checked-in app-server schema snapshot
- `docs/`: compact current-state docs and operator runbooks
- `scripts/`: packaging, validation, and repo helpers
- `examples/`, `.codex-exec-loop/followups/`: sample prompts and follow-up templates
