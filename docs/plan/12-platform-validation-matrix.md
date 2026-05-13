# Platform Validation Matrix

Use this matrix when a change affects terminal behavior:

- raw mode or restore handling
- inline behavior
- prompt editing, overlays, or shell chrome
- resize, scrollback, or visible cursor behavior

This matrix is about terminal behavior, not feature completeness.

## Frontend

- inline main-buffer only

## Common Commands

Build:

```bash
cd <path-to-repo>
. "$HOME/.cargo/env"
cargo build
```

Run:

```bash
cd <path-to-repo>
. "$HOME/.cargo/env"
cargo run
```

Record a validation row:

```bash
bash scripts/capture_native_validation.sh \
  --frontend inline \
  --check-profile terminal-baseline \
  --terminal "iTerm2 3.5" \
  --result pass \
  --output-dir docs/validation
```

Summarize recorded coverage:

```bash
bash scripts/summarize_native_validation.sh
```

## Minimum Matrix

| OS | Terminal | Shell | Frontend | Priority |
| --- | --- | --- | --- | --- |
| macOS | Terminal.app | zsh | inline | required |
| macOS | iTerm2 | zsh | inline | required |
| Windows | Windows Terminal | PowerShell | inline | required |
| Windows | Windows Terminal | WSL bash | inline | required |
| Windows | Git Bash or equivalent | bash | inline | optional |
| Windows | JetBrains IDE terminal | WSL bash | inline | optional |

## Check Profiles

### `terminal-baseline`

Use this profile for terminal-behavior changes such as raw mode, cursor restore, resize handling, prompt editing, or streaming behavior.

Run these checks once per required row:

1. Launch and clean exit
   - start the app
   - confirm the first frame renders cleanly
   - exit with `Ctrl+q`
   - confirm the shell prompt and cursor restore normally
2. Frontend baseline
   - run the default inline startup path
   - confirm the app always opens in inline main-buffer mode
3. Input editing
   - verify `Ctrl+j`, `Ctrl+u`, `Ctrl+w`, and `Enter`
   - confirm the prompt owns a visible cursor
4. Inspection flow
   - open diagnostics, sessions, queue, and planning at least once
   - close each surface with `Esc` or `Ctrl+C`
5. Streaming and status
   - confirm streamed text changes before completion
   - buffer input during streaming
   - confirm routine status hides raw ids and stays readable
6. Resize and scrollback
   - resize narrower, wider, shorter, and taller
   - in inline mode, inspect scrollback after a completed turn
7. Failure and recovery
   - terminate the app during a live session if the change touched restore behavior
   - confirm the terminal returns to a usable state

### `phase1-operator-surface`

Use this profile when a change touches:

- compact status wording or next-action copy
- queue, automation, planning, or directions operator surfaces
- session resume context
- external `akra doctor`, `akra status`, `akra queue`, `akra reset`
- in-shell `:doctor`, `:planning`, `:queue`, `:reset`

Record these rows with:

```bash
bash scripts/capture_native_validation.sh \
  --frontend inline \
  --check-profile phase1-operator-surface \
  --terminal "iTerm2 3.5" \
  --result pass \
  --output-dir docs/validation
```

Run the full `terminal-baseline` checklist plus these additional checks:

8. Status language and next action
   - confirm the compact shell status stays in operator vocabulary such as `ready`, `waiting`, `paused`, `blocked`, `repairing`, or `review needed`
   - confirm the visible status text names the next action when the shell is paused or blocked
   - confirm routine copy avoids raw internal ids or implementation-only terms
9. Resumed session context
   - load an existing session in a workspace with accepted planning
   - confirm the shell immediately surfaces planning status and queue summary after the thread loads
10. Queue and continuation explanation
   - open the queue and planning surfaces
   - confirm they explain current state, cause, and next action in operator language
   - confirm executable work, proposals, and blocked work read like work framing rather than file dumps
11. Lifecycle command parity
   - exercise the relevant external command path with `akra doctor`, `akra status`, `akra queue`, and the applicable `akra reset <target>`
   - exercise the matching in-shell path with `:doctor`, `:planning`, `:queue`, and the matching `:reset <target>`
   - confirm both command surfaces report the same lifecycle state and safety expectation

### `prompt-input-delay-pty`

Use this profile when prompt echo latency or input buffering changes, especially when the TUI runs
inside a PTY bridge, multiplexer, or integrated terminal.

Record these rows with:

```bash
bash scripts/capture_native_validation.sh \
  --frontend inline \
  --check-profile prompt-input-delay-pty \
  --terminal "tmux 3.4 detached PTY" \
  --result pass \
  --output-dir docs/validation
```

Summarize this profile separately from the broad terminal baseline:

```bash
bash scripts/summarize_native_validation.sh --check-profile prompt-input-delay-pty
```

Required rows:

| OS | Terminal | Shell | Frontend | Priority |
| --- | --- | --- | --- | --- |
| Linux | direct terminal | bash | inline | required |
| Linux | tmux detached PTY | bash | inline | required |
| Linux | Zellij | bash | inline | required |
| Windows | Windows Terminal | PowerShell | inline | required |
| Windows | Windows Terminal | WSL bash | inline | required |

Optional rows:

| OS | Terminal | Shell | Frontend | Priority |
| --- | --- | --- | --- | --- |
| macOS | Terminal.app | zsh | inline | optional |
| macOS | iTerm2 | zsh | inline | optional |
| Windows | JetBrains IDE terminal | WSL bash | inline | optional |
| Linux | VS Code integrated terminal | bash | inline | optional |

Run these checks once per required row:

1. Launch with the detached PTY backend or the terminal bridge that reproduces the latency-sensitive path.
2. Confirm prompt input echoes without visible delay during startup-pending state.
3. Confirm `Ctrl+u`, `Ctrl+w`, cursor movement, and multiline input stay responsive before submit.
4. Submit a prompt and confirm the shell transitions into streaming output without losing buffered input.
5. Confirm completion preserves prompt history and restores the cursor.
6. Interrupt or exit after delayed input checks and confirm terminal recovery remains responsive.

## Record Format

Each completed row should capture:

- date
- commit SHA
- OS
- terminal app and version
- shell
- frontend
- `TERM` when available
- check profile
- result and notes

Committed validation rows live under [`../validation/`](../validation/).
