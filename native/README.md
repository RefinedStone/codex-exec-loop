# native

Rust TUI client prototype for `codex-exec-loop`.

This crate is the main product track for `codex-exec-loop`.

Implemented baseline:

- spawn `codex app-server` over stdio
- perform `initialize`
- check account/auth state
- render a startup dashboard
- load recent sessions with `thread/list`
- load selected thread history with `thread/read`
- resume a selected thread and send a prompt with `turn/start`
- stream agent message deltas into the shell view
- queue a manual prompt while startup checks are still pending, then auto-send once startup becomes ready
- continue work automatically with builtin follow-up strategies from the TUI
- edit max auto turns, reload workspace templates, and stop auto follow-up with shell controls
- search, page, and filter recent sessions by current project context
- surface approval, tool activity, warning, and GitHub review-change notices in shell status
- schedule optional GitHub PR review/comment polling for the active or configured pull request

Protocol shape is pinned with a checked-in schema snapshot under `schema/`.

## Packaging

Build a distributable native bundle from the repository root:

```bash
./scripts/package_native_release.sh
```

The script stages a bundle under `dist/native/`, copies this crate README plus an operator runbook, writes a `.tar.gz` archive for handoff, and emits checksum files for both the unpacked bundle and the archive.

Verify a generated package:

```bash
./scripts/verify_native_release.sh \
  --archive dist/native/codex-exec-loop-native-<version>-<target>.tar.gz \
  --bundle-dir dist/native/codex-exec-loop-native-<version>-<target>
```

Capture a validation report scaffold after a manual terminal pass:

```bash
./scripts/capture_native_validation.sh --frontend inline --result pass
```

Use `--target <triple>` when the local Rust toolchain already supports that target. For Windows packaging, prefer running the script on Windows instead of assuming cross-linking from another OS.

Operator-facing packaging notes live in [`docs/plan/13-native-packaging-and-operator-runbook.md`](./docs/plan/13-native-packaging-and-operator-runbook.md).

## Optional GitHub Polling

The runtime can poll one GitHub pull request in the background and keep the polling state in the shell footer.

When RefinedStone GitHub credentials are available, the runtime first tries to auto-detect the current branch's open `prerelease` pull request. An explicit `CODEX_EXEC_LOOP_GITHUB_PR` value overrides that detection.

- `CODEX_EXEC_LOOP_GITHUB_PR=owner/repo#123`: enable polling for a specific pull request
- `CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS=60`: optional polling interval in seconds; defaults to `60`

When the poller is enabled, the footer shows whether polling is starting, running, watching normally, or blocked by a setup/runtime error, and it can surface compact review-change notices.

## Frontend Selection

The native shell defaults to inline main-buffer mode.

- `CODEX_EXEC_LOOP_FRONTEND=inline`: explicit inline main-buffer mode
- `CODEX_EXEC_LOOP_FRONTEND=alternate`: explicit fullscreen mode
- `CODEX_EXEC_LOOP_ALT_SCREEN=1`: legacy alternate-screen fallback

Prefer `CODEX_EXEC_LOOP_FRONTEND` for new operator docs and launch scripts.

## Architecture

The native crate prefers a Spring Boot Kotlin style hexagonal layout.

- `domain`
- `application/service`
- `application/port`
- `adapter/inbound`
- `adapter/outbound`
