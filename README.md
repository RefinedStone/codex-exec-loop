# codex-exec-loop

`codex-exec-loop` is a Rust TUI client built on `codex app-server`.

The repository root is now the product root. Run the app, tests, packaging helpers, docs, and schema snapshot from here.

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

- `CODEX_EXEC_LOOP_FRONTEND=inline`: inline main-buffer mode
- `CODEX_EXEC_LOOP_FRONTEND=alternate`: fullscreen alternate-screen mode
- `CODEX_EXEC_LOOP_ALT_SCREEN=1`: legacy alternate-screen fallback

Optional GitHub polling:

- `CODEX_EXEC_LOOP_GITHUB_PR=owner/repo#123`
- `CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS=60`

## Current Capability

- startup diagnostics and draft shell entry
- recent session browse, search, paging, and current-project filter
- existing thread resume, new thread start, and `turn/start` streaming
- startup-pending manual prompt queue then auto-submit
- inline inspections for diagnostics, sessions, and follow-up templates
- builtin and workspace follow-up templates, reload, editable max turns, and stop rules
- approval, tool activity, runtime warning, and GitHub review-change visibility
- packaging, checksum, and validation helper scripts

## Docs

Start here:

- [docs/README.md](./docs/README.md)
- [docs/design/01-current-product-state.md](./docs/design/01-current-product-state.md)
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

Capture a validation report scaffold:

```bash
./scripts/capture_native_validation.sh \
  --frontend inline \
  --result pass \
  --output-dir docs/validation
```

Summarize recorded validation rows:

```bash
./scripts/summarize_native_validation.sh
```

Render the summary as markdown:

```bash
./scripts/summarize_native_validation.sh --format markdown
```

## Repository Guide

- `src/`: Rust application code
- `schema/`: checked-in app-server schema snapshot
- `docs/`: design, plan, validation, and operator notes
- `scripts/`: packaging, validation, and repo helpers
- `examples/`, `.codex-exec-loop/followups/`: sample prompts and follow-up templates
