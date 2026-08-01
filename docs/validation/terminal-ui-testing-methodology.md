# Terminal UI Testing Methodology

The executable inventory is [`tui-coverage-matrix.md`](./tui-coverage-matrix.md). Akra's native
shell is one alternate screen application, not a host-scrollback formatter. Validation therefore
proves state, ordered frames, terminal lifecycle, and real-terminal restoration separately.
This document is paired with `docs/validation/tui-coverage-matrix.md`.

## Fullscreen Default and Ownership

The Ratatui/Crossterm fullscreen stack remains the default posture. There is no runtime override
for an inline renderer, host scrollback delivery, transcript replay mode, or newline-insertion
fallback.

- `NativeTuiApp` owns the canonical conversation and the app-owned transcript viewport.
- `ConversationViewModel.messages` is the only ordered transcript.
- `TranscriptViewportUiState` owns top row, follow-tail, unseen revision, document identity, and
  clickable card geometry.
- `FullscreenShellFrameModel` and `FullscreenInspectionFrameModel` are immutable owned draw input.
- `FullscreenFrameRenderReceipt` is the Stable frame receipt and compare-and-apply boundary.
- The Thin terminal layer enters the alternate screen, captures one sample, draws one frame, checks
  resize stability, and only then commits the receipt.
- The Render/layout boundary does no application, filesystem, network, clock, or terminal I/O.

### Environment key

- **E1** = Windows Terminal + WSL bash + fullscreen
- **E2** = Windows Terminal + PowerShell + fullscreen
- **E3** = tmux detached PTY + fullscreen
- **E4** = direct Linux terminal + fullscreen

The proof join is invariant × first-class environment. macOS Terminal.app and iTerm2 remain
representative compatibility targets in `docs/plan/12-platform-validation-matrix.md`.

## Responsibility Summary

| Responsibility | Current owner / source | Decision point | First-class default | Proof obligation |
|---|---|---|---|---|
| Semantic ordering | `ConversationViewModel.messages` | app-server event reduction | one canonical vector | agent/tool/agent order never changes after late completion |
| Reader position | `TranscriptViewportUiState` | input reducer + frame receipt | follow tail until user scrolls | streaming append does not move a reader |
| Rendering | `FullscreenShellFrameModel` | frame capture | owned immutable model | repeated draw has no effects |
| Delivery | `FullscreenTerminalAdapter` | post-draw geometry check | one Ratatui transaction | stale resize applies no UI receipt |
| Terminal modes | `TerminalRestoreGuard` | startup/drop | alternate screen + mouse/focus/paste | every enabled mode is restored |

The current stack remains the default posture because it minimizes bug-class recurrence across terminal boundaries,
future test-growth cost, and maintainability cost.

## Deterministic Layers

1. Reducer tests prove message ordering, thread identity, viewport movement, follow-tail, unseen
   output, tool-card expansion, and diff semantics.
2. Ratatui `TestBackend` tests render full frames at representative sizes. They assert the composer
   remains visible, old rows remain readable, expanded cards stay in the transcript, and semantic
   diff rows use the intended style.
3. Transaction tests prove the stable frame receipt applies once and a resize race applies nothing.
4. Lifecycle tests assert alternate-screen escape sequences, focus/mouse/paste modes, cursor
   restoration, and best-effort cleanup ordering.
5. Static architecture tests prove renderers cannot reacquire application authority and retired
   host delivery files cannot return unnoticed.

Temporal tests must compare sequential states. The core invariant is: **streaming append does not
move a reader**. A final screenshot alone cannot prove it.

## Architectural Guardrails

- Conversation, tool/read/explore, status, and assistant rows share one app-owned fullscreen viewport.
- There is no host scrollback delivery and no transcript handoff ACK.
- Parallel focused events use a dedicated fullscreen stream renderer and the typed
  `FullscreenAppendOnlyStream` surface.
- The parallel title may collapse without splitting the stream; event rows remain semantic data.
- `Ctrl+E` and mouse click expand the same card identity. PageUp/PageDown and wheel scroll the same
  viewport. `Ctrl+Home` leaves follow-tail; `Ctrl+End` resumes it.
- A thread identity change resets viewport offset and stale hit areas atomically.

## Manual Capture Contract

Manual terminal capture stays primitive-sensitive only. It is required when alternate-screen
escape sequences, viewport mode, clear or restore behavior, mouse capture behavior, terminal
wrapping, cursor placement, or resize primitives change. Copy-only and pure reducer changes use
deterministic tests unless a reviewer asks for more evidence.

### Reviewer gate

Capture must show:

1. the fullscreen transcript and composer at 80×24 or narrower;
2. PageUp while streaming, with the reader anchor unchanged and a `new output` badge;
3. Ctrl+End returning to the newest row;
4. a collapsed read/explore card and its expanded detail;
5. a collapsed patch card and expanded green/red semantic diff;
6. clean shell restoration after exit.

### When all first-class environments are required

Run E1–E4 when terminal mode, resize, cursor, wrapping, mouse, or restoration code changes.

### When a representative set is sufficient

Use one Windows-family and one Linux-family environment for presentation-only changes. Record why
the smaller set cannot mask a terminal primitive regression.

## Release Commands

```bash
cargo fmt --all -- --check
cargo test --lib -- --test-threads=1
cargo test --test architecture_boundaries -- --test-threads=1
cargo clippy --all-targets --all-features -- -D warnings
bash scripts/check_native_pr.sh
```
