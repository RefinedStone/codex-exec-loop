# Validation Records

This directory stores real terminal-validation results for the matrix in [`../plan/12-platform-validation-matrix.md`](../plan/12-platform-validation-matrix.md).

## Parallel Mode Production Validation

The 2026-04-30 parallel-mode validation records were compacted here from the former
`parallel-mode-*.md` payload files. Keep one summary table unless a future run needs a raw emitted
terminal checklist.

| Slice | Evidence | Result |
| --- | --- | --- |
| Happy path | single queued slot result | `akra parallel-tick` pushes the slot branch, ensures a PR against `prerelease`, integrates the queued commit, closes the PR, and returns the slot to idle |
| Production queued result | task `prod-e2e-20260430`, PR `#553`, source commit `3e2d3276d9cd2bed6409634799c237964e83b4b1`, integration commit `cf592f7b94ecac1bda4fba7f0d3090d17ca69960` | live run integrated the result and returned `slot-1` to idle; the first attempt exposed a stale remote branch collision, now covered by branch allocation checks |
| Restart recovery | task `prod-restart-e2e-20260430`, PR `#555`, source commit `aae7a55c5846f9881bbc0fedbc0d4a9fcba7773d`, integration commit `6efc46e816fa408f086180cd4abe9927e07e2e4d` | fresh `akra parallel-tick` recovered a store-backed `pr_pending` queue head, reused the existing PR, integrated it, closed it, and returned `slot-1` to idle |
| Blocked distributor | task `prod-blocked-e2e-20260430`, PR `#558`, conflict seed `ebc4418a825862f020d7085dcc81ca3f0776e448`, source commit `9d346be1cdb1831df6b7c0c507514b292c3cc682` | tick pushed the branch, opened the PR, attempted integration, aborted the failed cherry-pick, persisted the conflict file list, and left the queue head blocked for operator recovery |
| Multi-worktree | baseline `04a5e3c5472f5c89542a16c1500a6c082ef169dd`, PRs `#560` and `#561`, commits `e5006049323f8e046538472cbaff4d93d91579ea` and `24482aee13ed765c45e336a4920da19f67fb23f8` | two concurrently leased slots were processed one queue head at a time; `slot-1` and `slot-2` returned to idle and `slot-3` stayed available |

## Rules

- record only real passes or blockers
- keep one file per exercised terminal/frontend row, or one compact table for related production
  validation slices
- preserve the emitted `check_profile` and checklist so rows stay comparable
- use `bash scripts/summarize_native_validation.sh --fail-on-incomplete` when the matrix must act as a gate; the plain summary is informational and warns on incomplete required rows
- `scripts/capture_native_validation.sh` and `scripts/capture_native_validation.ps1` emit the shared baseline fields (`date`, `commit`, `os`, `terminal`, `shell`, `frontend`, `term`, `capture_role`, `check_profile`, `checks`, `result`, `notes`).
- Primitive-sensitive supplemental captures may append manual metadata below that helper output when review needs environment class, render/insertion mode, override stamps, or per-scenario results that the helpers do not emit directly.
- Supplemental primitive-sensitive captures may stay outside `scripts/summarize_native_validation.sh` entirely, or appear under `Unmatched Records`, depending on their `check_profile` and row metadata.

## Current Status

Current checked-in validation coverage is not matrix-complete yet.
Treat the plain summary output as a visibility report, not a release gate, until all required rows pass.

### `terminal-baseline` snapshot

| Metric | Current value |
| --- | --- |
| Required pass | `0/4` |
| Required missing | `4` |
| Required non-pass | `0` |
| Optional pass | `0/7` |

No required terminal-baseline row is currently recorded as passing.
The replay-policy captures below are supplemental representative evidence and do not count toward the baseline row totals.

### Supplemental replay-policy captures

- `docs/validation/2026-07-05-microsoft-windows-11-wsl-ubuntu-windows-terminal-wsl-bash-inline-replay-policy.txt` — explicit `ViewportReplay` representative evidence for the replay-only primitive change
- `docs/validation/2026-07-03-ubuntu-24-04-2-lts-wsl2-tmux-3-4-detached-pty-bash-inline-replay-policy.txt` — contrasting `HostScrollback` representative evidence for the unchanged branch family

### `prompt-input-delay-pty` snapshot

| Metric | Current value |
| --- | --- |
| Required pass | `1/5` |
| Required missing | `0` |
| Required non-pass | `4` |
| Optional pass | `0/4` |

Current required blockers still recorded in-tree:
- `Linux / direct terminal / bash / inline`
- `Linux / Zellij / bash / inline`
- `Windows / Windows Terminal / PowerShell / inline`
- `Windows / Windows Terminal / WSL bash / inline`

Use `bash scripts/summarize_native_validation.sh --format markdown` (and `--check-profile prompt-input-delay-pty --format markdown`) to refresh these numbers after new captures land.

## Filename Shape

Standard matrix-row capture:

```text
YYYY-MM-DD-<os>-<terminal>-<shell>-<frontend>.txt
```

Primitive-sensitive capture with scenario suffix:

```text
YYYY-MM-DD-<os>-<terminal>-<shell>-<frontend>-<scenario>.txt
```

Use `capture_role: counted-row` when the artifact body still names an exact matrix row and should count in `scripts/summarize_native_validation.sh`.
Use `capture_role: supplemental-unmatched` when the artifact is representative-only and may remain unmatched in the summary.

## Helper Usage

```bash
bash scripts/capture_native_validation.sh \
  --frontend inline \
  --check-profile terminal-baseline \
  --terminal "iTerm2 3.5" \
  --result pass \
  --output-dir docs/validation
```

Phase 1 operator-surface validation:

```bash
bash scripts/capture_native_validation.sh \
  --frontend inline \
  --check-profile phase1-operator-surface \
  --terminal "iTerm2 3.5" \
  --result pass \
  --output-dir docs/validation
```

Prompt input delay validation:

```bash
bash scripts/capture_native_validation.sh \
  --frontend inline \
  --check-profile prompt-input-delay-pty \
  --terminal "tmux 3.4 detached PTY" \
  --result pass \
  --output-dir docs/validation
```

Replay-policy representative capture:

```bash
CODEX_EXEC_LOOP_INLINE_HISTORY_MODE=viewport-replay \
  bash scripts/capture_native_validation.sh \
  --frontend inline \
  --check-profile replay-policy-representative \
  --capture-role supplemental-unmatched \
  --terminal "Windows Terminal" \
  --result pass \
  --output docs/validation/<date>-<os>-<terminal>-<shell>-<frontend>-replay-policy.txt
```

Then append the supplemental metadata required by `terminal-ui-testing-methodology.md`, including the effective render/insertion modes, exact env cause (`CODEX_EXEC_LOOP_INLINE_HISTORY_MODE`, `WT_SESSION` when relevant), and per-scenario results.

Windows PowerShell:

```powershell
.\scripts\capture_native_validation.ps1 `
  -Frontend inline `
  -CheckProfile terminal-baseline `
  -Terminal "Windows Terminal 1.22" `
  -Result pass `
  -OutputDir docs\validation
```

Coverage summary (informational; warns when required rows are incomplete):

```bash
bash scripts/summarize_native_validation.sh
```

Coverage gate:

```bash
bash scripts/summarize_native_validation.sh --fail-on-incomplete
```

Markdown summary:

```bash
bash scripts/summarize_native_validation.sh --format markdown
```

Prompt input delay summary:

```bash
bash scripts/summarize_native_validation.sh --check-profile prompt-input-delay-pty
```
