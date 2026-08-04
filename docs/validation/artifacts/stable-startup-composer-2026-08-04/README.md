# Stable Startup Composer Evidence

This focused artifact records the deterministic production-renderer proof for the startup-to-first-
turn layout transition. The welcome logo, the first edited draft, and the first submitted prompt all
use one fullscreen flow layout with the composer ending on physical row 23 of an 80×24 terminal.

## Result

| Frame | Body state | Composer bottom | Result |
| --- | --- | ---: | --- |
| Welcome | startup logo, empty draft | 23 | pass |
| Editing | startup logo, populated draft | 23 | pass |
| Active | submitted prompt in transcript | 23 | pass |

The row index is zero-based. The startup logo remains transcript/body content; it does not own a
second layout or move the status/composer tail.

## Design References

- [Codex conversation rendering](https://raw.githubusercontent.com/openai/codex/main/codex-rs/tui/src/chatwidget/rendering.rs)
  gives the transcript flexible height and the bottom pane a fixed-height slot.
- [Grok Build agent layout](https://github.com/xai-org/grok-build/blob/ba76b0a683fa52e4e60685017b85905451be17bc/crates/codegen/xai-grok-pager/src/views/agent.rs#L203)
  reserves explicit budgets for scrollback, queue, and input instead of switching layout after
  submission.
- [GJC runtime internals](https://github.com/Yeachan-Heo/gajae-code/blob/8132409c3f10754fea5f3b0108a7bee979c43652/docs/tui-runtime-internals.md)
  wires a persistent chat/status/editor component tree once and updates its content in place.

Akra keeps its existing Ratatui architecture and adopts the common geometry principle: one
persistent conversation frame, flexible body, fixed input tail.

## Evidence

- [Three-state rendered comparison](stable-composer-three-state-80x24.png)
- [Welcome frame](welcome-empty-80x24.txt)
- [Editing frame](welcome-editing-80x24.txt)
- [Active-conversation frame](active-conversation-80x24.txt)
- [Reproducible PNG renderer](render_capture.py)

The text frames come from
`startup_editing_and_active_conversation_share_the_same_bottom_anchored_composer`, which enters the
same `draw_projected` renderer used by the native fullscreen TUI. The PNG reconstructs those exact
cell rows for review; the TestBackend coordinates and assertions are the executable authority.

## Reproduce

```powershell
$env:AKRA_CAPTURE_STABLE_STARTUP_COMPOSER = '1'
cargo test startup_editing_and_active_conversation_share_the_same_bottom_anchored_composer --lib -- --test-threads=1 --nocapture
python docs/validation/artifacts/stable-startup-composer-2026-08-04/render_capture.py
```
