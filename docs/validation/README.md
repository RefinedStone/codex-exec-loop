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
- use `bash scripts/summarize_native_validation.sh` before calling the matrix complete

## Filename Shape

```text
YYYY-MM-DD-<os>-<terminal>-<shell>-<frontend>.txt
```

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

Windows PowerShell:

```powershell
.\scripts\capture_native_validation.ps1 `
  -Frontend inline `
  -CheckProfile terminal-baseline `
  -Terminal "Windows Terminal 1.22" `
  -Result pass `
  -OutputDir docs\validation
```

Coverage summary:

```bash
bash scripts/summarize_native_validation.sh
```

Markdown summary:

```bash
bash scripts/summarize_native_validation.sh --format markdown
```

Prompt input delay summary:

```bash
bash scripts/summarize_native_validation.sh --check-profile prompt-input-delay-pty
```
