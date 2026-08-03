# Fullscreen interaction E2E evidence

This candidate-specific E3 capture uses the production `akra` binary in a real detached tmux PTY
with a deterministic owner-only synthetic Codex app-server. It proves that a 1,200-sample SGR drag
settles to the final pointer position, retains the semantic selection background, emits the exact
OSC 52 clipboard payload, and leaves Ctrl+C available as copy instead of session termination.

The same run injects 3,000 unused all-motion reports before ordinary keyboard input. The input is
visible within the recorded bound because composition drops hover motion that the TUI does not use.
The submitted prompt and composer remain application chrome; response text is the selectable
surface. Raw PTY bytes and isolated paths are intentionally not retained.

## Result

- 1,200 SGR drag samples settled and copied the latest semantic range in 102ms.
- 3,000 unused all-motion reports followed by ordinary input displayed the key in 137ms.
- The selected cells retained the observed `RGB(42,72,112)` background.
- Raw PTY inspection found the exact OSC 52 base64 payload for
  `SELECTABLE_RESPONSE_CANARY`.
- Ctrl+C copied the retained selection without terminating the session; Ctrl+Q exited with status
  0.

## Evidence

- [Rendered PTY frame](./frames/selection-copy-120x30.png)
- [Plain PTY frame](./frames/selection-copy-120x30.txt)
- [Scenario results](./scenario-results.txt)
- [Environment stamp](./environment-stamp.txt)

The PNG is rendered from tmux's ANSI-preserving `capture-pane -e` output. It is a visual
representation of the captured terminal cells, not a hand-authored mockup.
