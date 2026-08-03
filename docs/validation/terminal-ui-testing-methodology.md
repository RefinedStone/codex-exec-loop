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
- `TranscriptViewportUiState` owns top row, follow-tail, unseen revision, document identity,
  clickable card geometry, the stable rendered-cell map, and drag selection.
- `FullscreenShellFrameModel` and `FullscreenInspectionFrameModel` are immutable owned draw input.
- `FullscreenFrameRenderReceipt` is the Stable frame receipt and compare-and-apply boundary.
- `NativeTerminalEventIngress` is a composition-owned continuous reader. It exposes an opaque
  mailbox to the TUI and drops unused all-motion reports before they can form an input backlog.
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
| Reader position | `TranscriptViewportUiState` | input reducer + frame receipt | follow tail until user scrolls or selects | streaming append does not move a reader or active selection |
| Rendering | `FullscreenShellFrameModel` | frame capture | owned immutable model | repeated draw has no effects |
| Delivery | `FullscreenTerminalAdapter` | post-draw geometry check | one Ratatui transaction | stale resize applies no UI receipt |
| Input collection | `NativeTerminalEventIngress` | composition reader boundary | continuous while a frame writes | drag/key events leave the host queue promptly; unused hover motion is absent |
| Frame admission | `TuiFrameScheduler` | shell runtime | dirty-coalesced at a 16ms boundary | a burst paints the latest state, never a slow replay |
| Terminal modes | `TerminalRestoreGuard` | startup/drop | alternate screen + mouse/focus/paste | every enabled mode is restored |
| Clipboard | terminal UI effect pump | explicit drag or `:copy` | OSC 52; tmux passthrough when attached | UTF-8 payload is base64 encoded and never printed as cells |

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
6. Input-ingress tests prove collection crosses the redacted composition boundary, errors are
   delivered without panic leakage, and thousands of unused motion reports cannot queue ahead of a
   real key.

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
- Left down/drag/up uses the last committed rendered-cell map. A click still toggles the same tool
  digest; movement turns the gesture into selection and copies only on release.
- A ready burst may collapse consecutive drag coordinates to its newest point, but it must retain
  down/up and every non-drag event in order. `Ctrl+C` with an active selection is a copy chord and
  cannot reach conversation interruption, navigation, or process-exit handling.
- User/status rows, the composer, and transient overlay badges are non-selectable interaction
  chrome. Agent/tool output uses per-message semantic ranges, and a drag stays inside its anchor
  range even when the pointer crosses another surface.
- Plain `MouseEventKind::Moved` reports have no TUI meaning and are removed at input ingress. A
  16ms frame-admission floor coalesces dirty state while the reader continues collecting input.
- `:mouse off` and `AKRA_TUI_MOUSE_CAPTURE=off` skip mouse reporting so the emulator can own native
  selection; `:mouse on` restores app-owned pointer input without changing transcript ownership.
- Snapshot contract tests resume a generated Akra main-session user message and assert that only
  `user-prompt`, or nested `original-user-prompt`, reaches the transcript and session preview.

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
7. forward and reverse transcript drag, visible selection highlighting, and clipboard paste into a
   separate application;
8. `:mouse off` native terminal selection and `:mouse on` restoration.
9. resume the selected session and confirm no execution/reporting/manual-intake prompt contract is
   visible.
10. inject a high-rate SGR drag plus unused all-motion reports and record drag settle time, next-key
    visibility, selection RGB, OSC 52 payload, Ctrl+C behavior, and exit status. The checked-in E3
    reference is [`artifacts/grok-interaction-pipeline-2026-08-04/`](./artifacts/grok-interaction-pipeline-2026-08-04/).

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
