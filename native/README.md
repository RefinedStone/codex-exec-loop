# native

Rust TUI client prototype for `codex-exec-loop`.

This crate is the main product track for `codex-exec-loop`.

Current milestones:

- spawn `codex app-server` over stdio
- perform `initialize`
- check account/auth state
- render a startup dashboard
- load recent sessions with `thread/list`
- load selected thread history with `thread/read`
- resume a selected thread and send a prompt with `turn/start`
- stream agent message deltas into the shell view
- continue work automatically with a builtin next-task follow-up prompt
- switch builtin follow-up strategies from the TUI
- load workspace follow-up templates from `.codex-exec-loop/followups/`
- stop auto follow-up when the agent emits `AUTO_STOP` or when the no-file-change rule is enabled
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

Use `--target <triple>` when the local Rust toolchain already supports that target. For Windows packaging, prefer running the script on Windows instead of assuming cross-linking from another OS.

Operator-facing packaging notes live in [`docs/plan/13-native-packaging-and-operator-runbook.md`](./docs/plan/13-native-packaging-and-operator-runbook.md).

## Optional GitHub Polling

The runtime can poll one GitHub pull request in the background and keep the polling state in the shell footer.

When RefinedStone GitHub credentials are available, the runtime first tries to auto-detect the current branch's open `prerelease` pull request. An explicit `CODEX_EXEC_LOOP_GITHUB_PR` value overrides that detection.

- `CODEX_EXEC_LOOP_GITHUB_PR=owner/repo#123`: enable polling for a specific pull request
- `CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS=60`: optional polling interval in seconds; defaults to `60`

When the poller is enabled, the footer shows whether polling is starting, running, watching normally, or blocked by a setup/runtime error. Review-change notices are still a later UI slice.

## Frontend Selection

The native shell defaults to inline main-buffer mode.

- `CODEX_EXEC_LOOP_FRONTEND=inline`: explicit inline main-buffer mode
- `CODEX_EXEC_LOOP_FRONTEND=alternate-screen`: explicit fullscreen mode
- `CODEX_EXEC_LOOP_ALT_SCREEN=1`: legacy alternate-screen fallback

Prefer `CODEX_EXEC_LOOP_FRONTEND` for new operator docs and launch scripts.

## Architecture

The native crate prefers a Spring Boot Kotlin style hexagonal layout.

- `domain`
- `application/service`
- `application/port`
- `adapter/inbound`
- `adapter/outbound`
