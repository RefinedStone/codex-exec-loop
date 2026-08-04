# PR 2071 Transcript Selection And Copy Evidence

> Historical evidence only. This directory records behavior at the named older commits, including
> interaction modes that have since been removed. It is not a current product contract or a
> supported operator-control reference.

This supplemental artifact records primitive-sensitive validation captured at foundational
implementation commit `1f867fd52a255ce85ef68a51d2d3a05d8e8ef311`. It exercises app-owned
transcript drag selection, tmux OSC 52 clipboard delivery, live mouse-capture switching, startup
native-selection mode, and terminal restoration against an actual tmux 3.4 detached PTY and the
production `akra` runtime. Post-review behavior commit
`f860af193616317b346b520e5348952f2003c0eb` keeps that terminal protocol path unchanged and adds
automated coverage for final-overlay snapshots, exact-width wrap separators, and wide-glyph cells.

The artifact is `supplemental-unmatched`: it is a candidate-specific first-class E3 observation.
It does not relabel E1, E2, or E4 as candidate passes.

## Result

- A real user transcript row was rendered by the production fullscreen shell at 80x24.
- SGR mouse down/drag selected exactly `selectable`; the committed frame displayed selection
  background `RGB(42,72,112)` without replacing the foreground style.
- Mouse release emitted tmux DCS passthrough containing OSC 52 target `c` and base64 payload
  `c2VsZWN0YWJsZQ==`, which decodes to the selected ten characters.
- The retained highlight remained visible and the shell status changed to
  `copied selection to terminal clipboard (10 characters)`.
- `:mouse on` emitted the complete 1000/1002/1003/1015/1006 enable set; `:mouse off` emitted the
  matching disable set.
- A legacy startup override, since removed from the product, entered the alternate screen without
  emitting the 1000 or 1006 mouse-reporting enable sequence.
- Confirmed exit restored alternate screen, focus reporting, and cursor visibility.

## Evidence Matrix

| Row | Candidate-specific manual result | Supporting evidence | Release-evidence treatment |
| --- | --- | --- | --- |
| E1 Windows Terminal + WSL bash | not executed | Windows targeted tests and prior fullscreen evidence | historical/automated support only |
| E2 Windows Terminal + PowerShell | not executed | Windows Crossterm lifecycle and interaction tests passed | automated support only |
| E3 tmux detached PTY | pass | this artifact, exact implementation commit and binary | candidate-specific first-class observation |
| E4 direct Linux terminal | not executed | Linux Clippy, architecture guard, TestBackend, and direct OSC encoder tests passed | explicit candidate-specific evidence gap |

## Files

- [environment-stamp.txt](environment-stamp.txt) records candidate and environment provenance.
- [scenario-results.txt](scenario-results.txt) records the manual sequence and exact escape-level
  assertions.
- [frames/selection-and-copy-80x24.txt](frames/selection-and-copy-80x24.txt) is a normalized,
  credential-free view of the selected frame and post-copy status.

## Scope And Safety

- The proof worktree, target directory, and tmux sessions were isolated under `/tmp`.
- The runtime used the actual Codex app-server startup path. Planning intake was intentionally
  unavailable in the isolated worktree, so no accepted task or durable planning mutation occurred.
- Raw PTY bytes are omitted; the exact escape markers and payload assertions are recorded in
  `scenario-results.txt`.
- No credentials, tokens, provider response, session identifier, or private prompt are included.
- No claim follows for Admin, CLI, Telegram, persistence, or released-package behavior.
