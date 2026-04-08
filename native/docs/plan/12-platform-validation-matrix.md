# Platform Validation Matrix

This file is the canonical manual validation guide for native terminal behavior on macOS and Windows.

Use it when a PR changes:

- raw-mode handling
- inline vs alternate-screen frontend behavior
- terminal restore behavior on success or failure
- input editing, overlays, or shell chrome

## Goal

Validate that the native client:

- starts in both supported frontend modes
- restores the terminal cleanly on exit and failure
- keeps input, streaming, and overlay flows usable across target terminals
- does not regress scrollback or resize behavior in platform-specific ways

## Scope

This matrix focuses on terminal behavior, not product feature completeness.

Out of scope:

- packaging installers
- release signing
- platform-specific filesystem bugs unrelated to terminal handling

## Frontend Modes

The native client currently supports two frontend modes:

- inline main-buffer mode
  - default mode
  - explicit env: `CODEX_EXEC_LOOP_FRONTEND=inline`
- alternate-screen mode
  - explicit env: `CODEX_EXEC_LOOP_FRONTEND=alternate`
  - legacy env fallback: `CODEX_EXEC_LOOP_ALT_SCREEN=1`

## Common Commands

Build once before a validation pass:

```bash
cd <path-to-repo>/native
. "$HOME/.cargo/env"
cargo build
```

Run in inline mode:

```bash
cd <path-to-repo>/native
. "$HOME/.cargo/env"
CODEX_EXEC_LOOP_FRONTEND=inline cargo run
```

Run in alternate-screen mode:

```bash
cd <path-to-repo>/native
. "$HOME/.cargo/env"
CODEX_EXEC_LOOP_FRONTEND=alternate cargo run
```

Run with the legacy alternate-screen flag:

```bash
cd <path-to-repo>/native
. "$HOME/.cargo/env"
CODEX_EXEC_LOOP_ALT_SCREEN=1 cargo run
```

PowerShell equivalents:

```powershell
Set-Location <path-to-repo>\native
cargo build
$env:CODEX_EXEC_LOOP_FRONTEND = "inline"; cargo run
$env:CODEX_EXEC_LOOP_FRONTEND = "alternate"; cargo run
$env:CODEX_EXEC_LOOP_FRONTEND = $null
$env:CODEX_EXEC_LOOP_ALT_SCREEN = "1"; cargo run
```

Capture a validation note scaffold in bash:

```bash
cd <path-to-repo>
./scripts/capture_native_validation.sh \
  --frontend inline \
  --terminal "iTerm2 3.5" \
  --result pass \
  --output-dir native/docs/validation
```

When this helper runs inside WSL, it records the Windows host OS plus the WSL distro and prefers `TERMINAL_EMULATOR` when an IDE terminal does not expose `WT_SESSION`.

Capture a validation note scaffold in PowerShell:

```powershell
Set-Location <path-to-repo>
.\scripts\capture_native_validation.ps1 `
  -Frontend inline `
  -Terminal "Windows Terminal 1.22" `
  -Result pass `
  -OutputDir native\docs\validation
```

Committed validation records live under [`../validation/`](../validation/).

Summarize recorded coverage before closing a platform-facing PR:

```bash
./scripts/summarize_native_validation.sh
```

Emit a markdown summary when you want to paste the current matrix state into a PR or issue:

```bash
./scripts/summarize_native_validation.sh --format markdown
```

## Environment Capture

Record this before marking a matrix row complete:

- date
- git commit SHA
- OS version
- terminal app and version
- shell
- frontend mode
- `TERM` value when available
- notes on any rendering or restore anomaly

## Minimum Matrix

These are the minimum environments that should be exercised before closing a platform-facing PR.

| OS | Terminal | Shell | Frontend | Priority |
| --- | --- | --- | --- | --- |
| macOS | Terminal.app | zsh | inline | required |
| macOS | Terminal.app | zsh | alternate | required |
| macOS | iTerm2 | zsh | inline | required |
| macOS | iTerm2 | zsh | alternate | required |
| Windows | Windows Terminal | PowerShell | inline | required |
| Windows | Windows Terminal | PowerShell | alternate | required |
| Windows | Windows Terminal | WSL bash | inline | required |
| Windows | Windows Terminal | WSL bash | alternate | required |
| Windows | Git Bash or equivalent | bash | inline | optional |
| Windows | JetBrains IDE terminal | WSL bash | inline | optional |
| Windows | JetBrains IDE terminal | WSL bash | alternate | optional |

## Validation Checklist

Run each check once per required matrix row unless a row is explicitly marked not applicable.

### 1. Launch and Clean Exit

- start the app
- confirm the first frame renders without a broken cursor or immediate terminal corruption
- exit with `Ctrl+q`
- confirm the shell prompt returns on a clean line
- confirm typed characters echo normally after exit

Pass condition:
- no stuck raw mode
- no missing prompt
- no invisible cursor after exit

### 2. Frontend Mode Selection

- run once with `CODEX_EXEC_LOOP_FRONTEND=inline`
- run once with `CODEX_EXEC_LOOP_FRONTEND=alternate`
- run once with `CODEX_EXEC_LOOP_ALT_SCREEN=1`
- confirm explicit `CODEX_EXEC_LOOP_FRONTEND` wins over the legacy flag when both are set

Pass condition:
- inline mode stays in main buffer
- alternate mode clearly uses alternate screen and restores the previous screen on exit

### 3. Input Editing

- type a short prompt
- use `Ctrl+j` to insert a newline
- use `Ctrl+u` to clear buffered input
- type multiple words and use `Ctrl+w` to delete the previous word
- send a prompt with `Enter`

Pass condition:
- editing shortcuts behave predictably
- cursor position remains coherent
- no duplicated or stale input is left on screen

### 4. Overlay and Inline Inspection Flow

- open diagnostics with `Ctrl+d`
- open sessions with `Ctrl+o`
- open follow-up templates with `Ctrl+p`
- close each surface with `Esc` or `Ctrl+C`

Pass condition:
- each surface opens and closes without leaving terminal artifacts
- focus returns to the shell correctly
- alternate-screen restore still works after visiting overlays

### 5. Streaming and Status Visibility

- send a prompt that produces a visible streaming turn
- while streaming, type buffered input without sending it
- confirm footer or overlay status continues to update
- confirm warnings, approval status, and GitHub status do not erase the prompt buffer or break layout

Pass condition:
- streaming updates remain readable
- buffered input remains intact
- status areas do not collapse into unreadable wrapping

### 6. Resize and Scrollback

- resize the terminal narrower, then wider
- resize the terminal shorter, then taller
- in inline mode, scroll upward in terminal scrollback after a completed turn
- in alternate-screen mode, confirm the app redraws instead of corrupting previous content

Pass condition:
- no panic
- no permanently clipped footer or input block
- inline mode leaves readable historical output in scrollback

### 7. Failure and Recovery

- run with an intentionally broken `codex` setup if practical, or trigger a startup failure path
- confirm diagnostics or warning text is readable
- exit after the failure path
- rerun once in a healthy environment

Pass condition:
- failure messaging is visible
- terminal state still restores cleanly after failure
- a healthy rerun does not inherit a broken terminal state

## Result Template

Use this block when recording a completed row in a PR or issue:

```text
date:
commit:
os:
terminal:
shell:
frontend:
term:
checks:
- launch and exit
- frontend selection
- input editing
- overlay flow
- streaming visibility
- resize and scrollback
- failure and recovery
result:
notes:
```

The helper scripts above emit this template with the current commit, detected host info, the standard checklist already filled in, and a slugged filename when `output-dir` is used.

Use `./scripts/summarize_native_validation.sh --fail-on-incomplete` when you want the shell to fail fast unless every required matrix row is recorded as `pass`.

## Exit Criteria

Platform validation is complete for a PR when:

- every required matrix row is marked pass or documented with a concrete blocker
- any blocker includes reproduction steps and terminal details
- follow-up Windows or macOS fixes are split into focused PRs instead of being left as vague notes

## Follow-up Rule

If validation finds a real platform bug:

1. open a focused follow-up branch from the latest `origin/prerelease`
2. reference this matrix row in the PR body
3. keep the fix scoped to the validated issue
