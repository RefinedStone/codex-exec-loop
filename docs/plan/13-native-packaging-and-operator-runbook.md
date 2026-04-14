# Native Packaging And Operator Runbook

This runbook describes the current bundle and operator handoff for the Rust client.

## Package Commands

Build the current host target:

```bash
cd /path/to/codex-exec-loop
./scripts/package_native_release.sh
```

Build a specific target:

```bash
cd /path/to/codex-exec-loop
./scripts/package_native_release.sh --target aarch64-apple-darwin
```

Notes:

- default build profile is `release`
- `--profile debug` is available for local validation bundles
- prefer native Windows packaging for Windows bundles instead of unvalidated cross-linking

## Output Layout

```text
dist/native/
  codex-exec-loop-native-<version>-<target>/
  codex-exec-loop-native-<version>-<target>.tar.gz
  codex-exec-loop-native-<version>-<target>.tar.gz.sha256
```

Bundle contents:

- native binary
- `akra` launcher on macOS/Linux or `akra.cmd` on Windows
- `README.md`
- `OPERATOR.md`
- `VERSION.txt`
- `examples/`
- `.codex-exec-loop/followups/`
- `SHA256SUMS.txt`

The npm distribution uses a Codex-style split:

- main package `akra`
- platform packages published as npm optional dependencies
- a tiny JavaScript launcher that resolves the installed native binary and executes it

The TUI itself still runs inside the Rust binary. JavaScript only acts as the npm entrypoint and platform selector.

## Integrity Verification

Verify both archive and unpacked bundle:

```bash
cd /path/to/codex-exec-loop
./scripts/verify_native_release.sh \
  --archive dist/native/codex-exec-loop-native-<version>-<target>.tar.gz \
  --bundle-dir dist/native/codex-exec-loop-native-<version>-<target>
```

The packaging flow emits:

- `SHA256SUMS.txt` for files inside the unpacked bundle
- `<archive>.tar.gz.sha256` for the tarball itself

## Operator Prerequisites

- Codex CLI installed and on `PATH`
- Codex login already completed
- access to the target workspace
- normal access to `~/.codex/history.jsonl` and `~/.codex/sessions/`

Rust is not required on the operator machine after the bundle is built.

## Launch

If the unpacked bundle directory is on `PATH`, launch from any workspace with:

macOS or Linux:

```bash
cd /path/to/workspace
akra
```

Windows PowerShell:

```powershell
Set-Location C:\path\to\workspace
akra
```

You can still run the native binary directly if you prefer:

macOS or Linux:

```bash
cd /path/to/workspace
/path/to/codex-exec-loop-native
```

Windows PowerShell:

```powershell
Set-Location C:\path\to\workspace
C:\path\to\codex-exec-loop-native.exe
```

Useful env vars:

- `CODEX_EXEC_LOOP_GITHUB_PR=owner/repo#123`
- `CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS=60`

## Smoke Checklist

1. Start the binary from a real workspace.
2. Confirm startup diagnostics pass.
3. Open recent sessions or start a new draft.
4. Send one prompt and confirm streaming output appears.
5. Open `:planning` once if planning is part of the workflow.
6. Open `:queue` once and confirm the compact queue summary appears.
7. If GitHub polling is expected, set `CODEX_EXEC_LOOP_GITHUB_PR` and confirm the shell shows an active GitHub state.

## Validation Handoff

After a platform-facing change, record the validation result instead of rewriting the matrix by hand:

```bash
./scripts/capture_native_validation.sh \
  --frontend inline \
  --terminal "iTerm2 3.5" \
  --result pass \
  --output-dir docs/validation
```

Coverage summary:

```bash
./scripts/summarize_native_validation.sh
```

Markdown summary:

```bash
./scripts/summarize_native_validation.sh --format markdown
```

## Release Notes

- package from a clean checkout when possible
- attach the exact commit SHA used for the build
- keep the emitted `.sha256` file with the archive
- fix platform-specific terminal defects in focused follow-up branches instead of widening the packaging change

## GitHub Release Assets

The repository can publish native bundles directly to GitHub Release assets from a tag push.

- workflow: `.github/workflows/release-native-assets.yml`
- accepted tags: any pushed tag
- published assets:
  - Linux `x86_64-unknown-linux-gnu`
  - Windows `x86_64-pc-windows-msvc`
  - macOS `aarch64-apple-darwin`
- each asset upload includes the archive and matching `.sha256` file
- asset file names still use the package version declared in `Cargo.toml`
- when `NPM_TOKEN` exists in repository secrets, the same tag also publishes:
  - `akra@<tag-version>`
  - `akra@<tag-version>-linux-x64`
  - `akra@<tag-version>-darwin-arm64`
  - `akra@<tag-version>-win32-x64`

npm publish notes:

- publish platform packages before the main `akra` package
- npm versions are immutable, so a corrected republish needs a new tag version
- the npm package ships only the native binary under `vendor/<target>/akra/`

Typical release flow:

```bash
git tag v0.1.0
git push origin v0.1.0
```

After the workflow finishes, download the archives from the GitHub `Releases` page for that tag.
