# Platform Validation Matrix

[한국어](../ko/reference/validation.md)

Use this matrix when a change affects raw mode, terminal restore, inline behavior, prompt editing,
overlays, resize, scrollback, or cursor behavior. It validates terminal behavior, not feature
completeness.

The shipped frontend is inline main-buffer mode. Counted `terminal-baseline` rows cover that default;
alternate-screen or branch-family evidence is supplemental and does not replace a required row.

## Required Terminal Baseline

| OS | Terminal | Shell | Frontend | Priority |
| --- | --- | --- | --- | --- |
| macOS | Terminal.app | zsh | inline | required |
| macOS | iTerm2 | zsh | inline | required |
| Windows | Windows Terminal | PowerShell | inline | required |
| Windows | Windows Terminal | WSL bash | inline | required |
| Windows | Git Bash or equivalent | bash | inline | optional |
| Windows | JetBrains IDE terminal | WSL bash | inline | optional |

Run once per required row:

1. launch, render the first frame, exit with `Ctrl+q`, and confirm prompt/cursor restoration
2. edit with `Ctrl+j`, `Ctrl+u`, `Ctrl+w`, cursor movement, multiline input, and `Enter`
3. open and close diagnostics, sessions, queue, and planning
4. stream a turn and verify buffered input and compact status behavior
5. resize narrower, wider, shorter, and taller; inspect committed host scrollback
6. exercise failure/interrupt recovery when terminal restoration changed

## Operator-Surface Profile

Use `phase1-operator-surface` when changing status wording, session-resume context, queue,
automation, planning/directions, or matching `akra`/`:` lifecycle commands. Run the baseline plus:

- verify operator vocabulary and an actionable next step for paused/blocked states
- load an existing session and confirm planning/queue context appears immediately
- compare `akra doctor|status|queue|reset` with `:doctor|:planning|:queue|:reset`
- keep routine copy free of raw internal IDs and implementation-only terms

## Prompt-Input-Delay Profile

Use `prompt-input-delay-pty` when prompt echo, input buffering, PTY bridges, multiplexers, or
integrated terminals change.

| OS | Terminal | Shell | Priority |
| --- | --- | --- | --- |
| Linux | direct terminal | bash | required |
| Linux | tmux detached PTY | bash | required |
| Linux | Zellij | bash | required |
| Windows | Windows Terminal | PowerShell | required |
| Windows | Windows Terminal | WSL bash | required |
| macOS | Terminal.app or iTerm2 | zsh | optional |
| Windows/Linux | IDE integrated terminal | WSL bash or bash | optional |

Verify startup-pending echo, editing/cursor responsiveness, multiline input, submit-to-stream
transition, prompt history/cursor restoration, and interrupt/exit recovery. Historical or
supplemental rows do not count as a new approval-grade pass.

## Capture and Summary

Build and run:

```bash
. "$HOME/.cargo/env"
cargo build
cargo run
```

Record one row:

```bash
bash scripts/capture_native_validation.sh \
  --frontend inline \
  --check-profile terminal-baseline \
  --terminal "iTerm2 3.5" \
  --result pass \
  --output-dir docs/validation
```

Use `phase1-operator-surface` or `prompt-input-delay-pty` as `--check-profile` when applicable.
PowerShell uses `scripts/capture_native_validation.ps1` with the corresponding parameters.

```bash
bash scripts/summarize_native_validation.sh
bash scripts/summarize_native_validation.sh --fail-on-incomplete
bash scripts/summarize_native_validation.sh --format markdown
bash scripts/summarize_native_validation.sh \
  --check-profile prompt-input-delay-pty --fail-on-incomplete
```

The plain summary is informational; `--fail-on-incomplete` is the explicit gate. Counted rows use
`capture_role: counted-row`. Representative branch-family evidence uses
`capture_role: supplemental-unmatched` and never waives a required row by itself.

Every record includes date, commit SHA, OS, terminal/version, shell, frontend, `TERM` when
available, capture role, exact profile, emitted checks, result, and notes. Primitive-sensitive
changes also follow [the TUI methodology](../validation/terminal-ui-testing-methodology.md) and may
require E1-E4 environment/mode metadata beyond the helper's baseline fields.

Committed records live under [docs/validation/](../validation/).
