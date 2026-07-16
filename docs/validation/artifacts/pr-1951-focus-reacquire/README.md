# PR 1951 Focus Reacquire Supplemental Evidence

This artifact records one focused, privacy-reviewed terminal observation for candidate commit
`03503158ce79a9e4c60a5b07635db054611af3d8` and tree
`7c76a01b5f5559cc85cc8fae73cdd1360e8e196b`.

It is `supplemental-unmatched` evidence. It is not an E1-E4 matrix row and does not satisfy the
primitive-sensitive approval gate by itself.

## Environment Stamp

- artifact id: `pr-1951-focus-reacquire`
- capture role: `supplemental-unmatched`
- pull request: `#1951`
- capture time: `2026-07-17 02:42:52 +0900`
- operator/reviewer: `Codex /root`
- frontend: `inline`
- environment class: supplemental macOS tmux path, not first-class E1-E4
- terminal: tmux `3.6a` detached PTY inside the Codex execution terminal
- shell: zsh `5.9 (arm64-apple-darwin25.0)`
- OS: macOS `26.5 (25F71)`, Darwin `25.5.0`, arm64
- multiplexer: tmux detached PTY with a private tmux server
- geometry: `100x30`, with an unfocused `48x18 -> 100x30` shrink/restore sequence
- `InlineHistoryRenderMode`: `HostScrollback`
- `HistoryInsertionMode`: `StandardScrollRegion`
- overrides: `CODEX_EXEC_LOOP_INLINE_HISTORY_MODE=scrollback`,
  `CODEX_EXEC_LOOP_HISTORY_INSERT_MODE=standard`, `TERM=xterm-256color`
- scenario set: focused focus-loss/clobber/reacquire observation
- result: focused scenario pass; matrix approval incomplete

## Observed Sequence

The source-built `target/debug/akra` reached a ready inline shell. A CJK draft was entered without
submission. The private tmux PTY then received the standard focus-lost sequence (`ESC [ O`), was
shrunk to `48x18` and restored to `100x30`, and had only its visible tail replaced with
`FOCUS_CLOBBER`. The PTY cursor was restored to its pre-clobber coordinate before sending the
focus-gained sequence (`ESC [ I`).

Before focus loss:

```text
Akra  |  Workflows: ready  |  Sessions: 10 loaded  |  Reviews: off
> 포커스 복귀 한글 증거
buffered prompt  |  Enter send  |  Ctrl+j nl
pane=100x30 cursor=23,7 history=0 dead=0
```

While unfocused, after shrink/restore and visible-tail replacement:

```text
Akra  |  Workflows: ready  |  Sessions: 10 loaded  |  Reviews: off
FOCUS_CLOBBER
pane=100x30 cursor=23,7 history=0 dead=0
```

After focus gain:

```text
Akra  |  Workflows: ready  |  Sessions: 10 loaded  |  Reviews: off
> 포커스 복귀 한글 증거
buffered prompt  |  Enter send  |  Ctrl+j nl
pane=100x30 cursor=23,7 history=0 dead=0
```

Observed assertions:

- the clobber marker disappeared on focus gain without another input event
- the exact CJK draft returned
- geometry returned to `100x30`
- the physical cursor returned to `23,7`
- no scoped tmux server or Akra process remained after capture cleanup

## Automated Proof Joined To This Observation

- `focus_reacquire_repaints_visible_frame_without_replaying_host_scrollback` records exact
  TestBackend frames and proves the durable host-scrollback baseline is unchanged.
- `vt100_focus_reacquire_repaints_once_across_cjk_resize` exercises Crossterm over VT100,
  `NewlineFallback`, CJK shrink/restore, the parser cursor, one backend draw for a true transition,
  and zero additional draws for a duplicate focus-gain event.
- `focus_lost_blocks_draw_until_focus_returns` proves the scheduler suppresses unfocused draws and
  increments the focus-reacquire epoch only on a real transition.

## Required Scenario Disposition

| Required scenario | Result in this artifact |
| --- | --- |
| committed history insertion above live viewport | not run; covered only by named automated tests |
| no duplicate replay after redraw | automated proof only |
| shrink then restore | pass in the detached PTY |
| clear/reset | not run |
| thread/session switch | not run |
| parallel/live-tail split | not applicable to this focused scenario |
| fallback insertion | VT100 automated proof only |
| focus-loss/reacquire after visible replacement | pass in the detached PTY |

## Limitations

- Focus events were injected as the standard terminal focus escape sequences; no GUI window switch
  or physical tmux detach/reattach was claimed.
- No provider turn was submitted, so this record contains no model output and no committed-history
  manual assertion.
- E1 Windows Terminal/WSL, E2 Windows Terminal/PowerShell, E3 Linux tmux, E4 direct Linux, and a
  representative `ViewportReplay` capture remain external approval work.
- The frame excerpts intentionally omit the workspace path, session identifiers, diagnostics, and
  authentication state.
