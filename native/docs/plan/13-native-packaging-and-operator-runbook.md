# Native Packaging and Operator Runbook

This runbook defines the current handoff path for the Rust native client.

The package target is the `codex-exec-loop-native` binary plus the minimum operator docs needed to run it on a machine that already has Codex CLI access.

## Package Command

Build and package the current host target:

```bash
cd /path/to/codex-exec-loop
./scripts/package_native_release.sh
```

Build and package a specific Rust target triple:

```bash
cd /path/to/codex-exec-loop
./scripts/package_native_release.sh --target aarch64-apple-darwin
```

Notes:

- the script builds `native/` with `cargo build --release` by default
- `--profile debug` is available for local validation bundles
- `--target` assumes the target toolchain and linker already work on the current machine
- for Windows bundles, prefer running the script on a Windows Rust toolchain instead of relying on unvalidated cross-linking from Linux or macOS

## Output Layout

Default output path:

```text
dist/native/
  codex-exec-loop-native-<version>-<target>/
  codex-exec-loop-native-<version>-<target>.tar.gz
  codex-exec-loop-native-<version>-<target>.tar.gz.sha256
```

Bundle contents:

- `codex-exec-loop-native` or `codex-exec-loop-native.exe`
- `README.md`
- `OPERATOR.md`
- `VERSION.txt`
- `SHA256SUMS.txt`

Archive sidecar:

- `codex-exec-loop-native-<version>-<target>.tar.gz.sha256`

## Integrity Verification

The packaging script emits two checksum artifacts:

- `SHA256SUMS.txt` inside the bundle directory for the unpacked files
- `<archive>.tar.gz.sha256` next to the release archive for the tarball itself

Examples:

Linux:

```bash
cd dist/native
sha256sum -c codex-exec-loop-native-<version>-<target>.tar.gz.sha256
```

macOS:

```bash
cd dist/native
shasum -a 256 -c codex-exec-loop-native-<version>-<target>.tar.gz.sha256
```

If the packaging machine only has `openssl`, compare the emitted digest file with `openssl dgst -sha256 <archive>`.

Repository helper:

```bash
cd /path/to/codex-exec-loop
./scripts/verify_native_release.sh \
  --archive dist/native/codex-exec-loop-native-<version>-<target>.tar.gz \
  --bundle-dir dist/native/codex-exec-loop-native-<version>-<target>
```

The helper validates the archive sidecar and each unpacked bundle artifact against the generated checksum files.

## Operator Prerequisites

The operator machine needs:

- Codex CLI installed and on `PATH`
- Codex login already completed
- access to the workspace that Codex should operate on
- normal access to `~/.codex/history.jsonl` and `~/.codex/sessions/`

Rust is not required on the operator machine when the packaged binary is already built.

## Launch Commands

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

## Useful Runtime Environment Variables

- `CODEX_EXEC_LOOP_FRONTEND=inline`
- `CODEX_EXEC_LOOP_FRONTEND=alternate`
- `CODEX_EXEC_LOOP_ALT_SCREEN=1`
- `CODEX_EXEC_LOOP_GITHUB_PR=owner/repo#123`
- `CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS=60`

`CODEX_EXEC_LOOP_FRONTEND` is the preferred frontend selector.

`CODEX_EXEC_LOOP_ALT_SCREEN=1` exists as a legacy fallback for alternate-screen mode.

## Operator Smoke Checklist

Run this checklist after copying the bundle to a target machine:

1. Start the binary from a real workspace.
2. Confirm startup diagnostics pass.
3. Open recent sessions or start a new draft.
4. Send one prompt and confirm streaming output appears.
5. If the machine should use fullscreen mode, set `CODEX_EXEC_LOOP_FRONTEND=alternate` and repeat the launch.
6. If GitHub polling is part of the workflow, set `CODEX_EXEC_LOOP_GITHUB_PR` and confirm the footer shows an active GitHub state instead of a setup error.

After a platform-facing validation pass, capture the result block with a helper instead of rewriting the matrix template by hand.

macOS or Linux:

```bash
cd /path/to/codex-exec-loop
./scripts/capture_native_validation.sh \
  --frontend inline \
  --result pass \
  --output-dir native/docs/validation
```

Windows PowerShell:

```powershell
Set-Location C:\path\to\codex-exec-loop
.\scripts\capture_native_validation.ps1 `
  -Frontend inline `
  -Result pass `
  -OutputDir native\docs\validation
```

Keep the recorded files under `native/docs/validation/` so later platform follow-ups can point to a checked-in row instead of a transient comment.

Check current matrix coverage from the repository root:

```bash
./scripts/summarize_native_validation.sh
```

For a copy-pastable PR summary:

```bash
./scripts/summarize_native_validation.sh --format markdown
```

## Release Handoff Notes

- keep package creation deterministic by running from a clean checkout
- attach the generated archive together with the exact commit SHA used to build it
- keep the generated `.sha256` file with the archive so the receiver can verify integrity before unpacking
- when platform-specific validation finds terminal defects, fix those in a focused follow-up branch instead of widening the packaging script
