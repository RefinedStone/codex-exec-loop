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
./scripts/capture_native_validation.sh \
  --frontend inline \
  --terminal "iTerm2 3.5" \
  --result pass \
  --output-dir docs/validation
```

Summarize recorded coverage:

```bash
./scripts/summarize_native_validation.sh
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

## Checklist

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
   - open diagnostics, sessions, automation controls, and planning at least once
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

## Record Format

Each completed row should capture:

- date
- commit SHA
- OS
- terminal app and version
- shell
- frontend
- `TERM` when available
- result and notes

Committed validation rows live under [`../validation/`](../validation/).
