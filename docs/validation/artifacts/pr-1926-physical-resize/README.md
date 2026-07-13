# PR 1926 Physical Resize Approval Evidence

This artifact set records the primitive-sensitive manual validation for candidate commit
`0a5f06ea345bd5c24998527679158eaf8643d058` and tree
`7e4c0cd0de3b6609a88786eefe58074d1072d5fd`. Every row ran a source build of that exact clean tree
against an actual Codex app-server runtime.

The records are `supplemental-unmatched` because they are outside the dated validation-summary
scenario set. They are nevertheless first-class, independently reviewed evidence for this PR's
shared terminal-primitive approval obligation. They are not released Akra runtime activity
evidence and do not change
the competitive capture's `approvalGrade: false` classification.

## Environment Matrix

| Row | Environment | History modes | Runtime flow | Geometry authority | Result |
| --- | --- | --- | --- | --- | --- |
| E1 | Windows Terminal 1.24, WSL Ubuntu, bash | `HostScrollback`, `NewlineFallback` | actual `sleep 5` command and exact final | live WSL PTY | pass |
| E2 | Windows Terminal 1.24, Windows PowerShell | `HostScrollback`, `NewlineFallback` | direct exact final, intentionally no tool call | Windows console API | pass |
| E3 | tmux 3.4 detached PTY, bash | `HostScrollback`, `StandardScrollRegion` | actual `sleep 5` command and exact final | tmux pane geometry/history | pass |
| E4 | xterm 390 via WSLg, bash | `HostScrollback`, `StandardScrollRegion` | direct exact final, intentionally no tool call | live `stty` geometry | pass |

All four rows cover multiline input editing, an active turn, one committed exact final, help
open/close redraw, physical shrink and restore, clear/new-draft reset, session restore, clean exit,
and clean runtime exit. E1, E2, and E4 also record terminal recovery; E3 records pane exit, tmux
server shutdown, and zero scoped processes. E1 and E2 shrink from 133x30 to 48x18 and restore to
133x30. E3 and E4 shrink from 120x30 to 48x18 and restore to 120x30.

## Evidence Index

- [E1 Windows Terminal/WSL](e1-windows-terminal-wsl/)
- [E2 Windows Terminal/PowerShell](e2-windows-terminal-powershell/)
- [E3 tmux detached PTY](e3-tmux/)
- [E4 direct xterm](e4-direct-xterm/)

Each environment contains normalized provenance, integrity, private-rollout structural results,
scenario results, selected safe frames or screenshots, and a local `SHA256SUMS` index. The root
`SHA256SUMS` covers the complete checked-in set.

## Scope And Limitations

- Akra was built from the candidate source tree; no released Akra package was used.
- E3 observed an account rate-limit notice and the Windows rows observed GitHub setup warnings.
  Those bounded notices did not change command, final, resize, restore, or exit assertions.
- Raw authentication state, rollouts, absolute paths, PTY identifiers, full task identifiers,
  session pickers, startup frames, transition frames, and capture helpers are excluded.
- Host scrollback intentionally retains durable committed history. Clear/reset assertions apply to
  the new live viewport and pending/deferred state, not to purging terminal-owned scrollback.
- No Admin, CLI, Telegram, persistence, recovery, performance, or comparative product claim follows
  from these captures.

## Review

Review status: pass. Independent reviewer `Codex /root/e1_e2_terminal_capability` found no Blocker
or P2 issue in the assembled artifact, documentation claims, integrity indexes, or privacy scan.
