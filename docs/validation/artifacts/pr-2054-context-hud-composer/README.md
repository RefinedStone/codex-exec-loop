# PR 2054 Context HUD And Composer Evidence

This supplemental artifact records the focused real-terminal validation for candidate commit
`bd18ab3cdf557e135921bb5ae625ec64ca1cf40c` and tree
`03b36f1033432162e85c1a7c1bfdf17044ce798b`.

The source-built Akra binary ran against an actual Codex `0.146.0` app-server in a private tmux
PTY. The observation covers the first commercial-UX slice: the compact context HUD, focused
composer, CJK cursor placement, responsive collapse, diagnostics open/close, and resize-reflow
cleanup.

## Result

| Scenario | Geometry | Result |
| --- | --- | --- |
| Ready HUD and Korean draft | `120x30` | pass |
| Physical shrink with the same draft | `48x18` | pass |
| Physical restore | `120x30` | pass |
| Diagnostics open and close | `120x30` | pass |
| Clean scoped exit | private tmux PTY | pass |

The first shrink observation exposed a real tmux reflow fragment from the former wide HUD. The
candidate was corrected before these final frames: ordinary conversation scrollback now reserves
bounded physical guard rows and the adapter clears only a confirmed guarded region plus
cursor-confirmed wrap rows. Those guards are excluded from transcript identity and parallel event
batches, so session replay and one-shot handoff delivery remain intact. The final narrow and
restored frames contain no old HUD fragment, duplicate ribbon, or stale diagnostics row.

## Critical Plan Adjustments

- Unknown branch and context values are omitted instead of rendering `branch: --` or `ctx: --`.
  The renderer performs no Git I/O; context appears only after an authoritative token-usage event.
- The image-generation concept's large closed composer was reduced to a three-row semantic open
  rail. This preserves the repository's native inline-main-buffer and host-scrollback contract.
- Two blank physical host rows separate ordinary durable transcript from the live tail. They
  provide deliberate breathing room and a terminal-safe reflow guard without becoming
  application-owned history or consuming the parallel operations scroll-region budget.

## Evidence

- [Wide CJK input](frames/01-wide-input-120x30.txt)
- [Clean narrow resize](frames/02-narrow-input-48x18.txt)
- [Clean restored width](frames/03-restored-input-120x30.txt)
- [Diagnostics open](frames/04-diagnostics-open-120x30.txt)
- [Diagnostics closed](frames/05-diagnostics-closed-120x30.txt)
- [Rendered capture comparison (PNG)](context-hud-composer-capture.png)
- [Rendered capture comparison (SVG source)](context-hud-composer-capture.svg)
- [Environment stamp](environment-stamp.txt)
- [Scenario results](scenario-results.txt)

## Automated Proof Joined To This Observation

- `shell_rendering::tests` covers the 80/120/160-column ready shell, typing, streaming,
  approval, blocked startup, parallel loading, and snapshots.
- `focused_composer_cursor_uses_terminal_cells_for_korean_and_wide_graphemes` covers CJK and
  grapheme cursor cells.
- `inline_terminal_adapter::tests` passed 84 serial cases covering history insertion, VT100
  behavior, focus reacquire, resize races, stale-row cleanup, guarded width transitions, and
  parallel handoff preservation.
- `shell_rendering::contract_tests` passed 58 cases, and `status_panels` passed 46 cases covering
  responsive rendering, state-specific action copy, and typed status priority.
- Linux `cargo clippy --locked --all-targets --all-features -- -D warnings` passed.

## Scope And Limitations

- This is first-class E3 terminal observation but remains `supplemental-unmatched`; it does not
  replace the complete E1-E4 approval matrix in `pr-1926-physical-resize`.
- E1 Windows Terminal/WSL, E2 Windows Terminal/PowerShell, and E4 direct xterm were not recaptured
  for this focused slice.
- The diagnostics frame is normalized to remove the absolute workspace path.
- No provider turn was submitted, so this artifact makes no model-output or performance claim.
- No Admin, API, persistence, Telegram, or parallel-delivery behavior is claimed here.
