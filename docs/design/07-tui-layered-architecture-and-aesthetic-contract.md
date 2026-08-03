# TUI Fullscreen Architecture and Aesthetic Contract

## Product Intent

Akra behaves like a modern fullscreen agent client: the transcript is the workspace, the composer
is always reachable, and operational detail is quiet until requested. This replaces the former
terminal-scrollback composition model completely.

The target is Grok-like fullscreen behavior adapted to Akra's orchestration vocabulary:

- one continuous conversation viewport;
- compact assistant/tool summaries with in-place expansion;
- readable semantic diffs inside the conversation;
- a fixed, low-noise status/composer tail;
- focused operational views that still use the same fullscreen frame transaction.

## Layer Contract

```text
app-server events
  → conversation/core reducers
  → ConversationViewModel.messages
  → FullscreenConversationFrameProjection
  → FullscreenShellFrameModel + FullscreenInspectionFrameModel
  → pure Ratatui draw
  → FullscreenFrameRenderReceipt
  → compare-and-apply UI state
```

`ConversationViewModel.messages` is the only transcript authority. Assistant streaming creates or
updates the row identified by `item_id`; tool rows append at arrival time. A late completion updates
the original assistant row in place and cannot reorder assistant/tool/assistant history.

`FullscreenShellFrameModel` is owned and lifetime-free. `FullscreenInspectionFrameModel` contains
the selected focused view. `FullscreenFrameRenderReceipt` carries only UI feedback that may be
committed after a stable terminal draw.

Terminal input is collected continuously by the composition-owned `NativeTerminalEventIngress`.
The TUI receives only its opaque event mailbox, never a thread or process capability. This keeps
mouse/key intake moving while Ratatui writes a frame, while all application reduction still occurs
in order on `ShellRuntime`.

## Fullscreen Layout

The frame has two vertical regions:

1. a flexible transcript or focused inspection body;
2. a bounded shell tail containing critical status and the composer.

The body gets the remaining height. The composer never disappears because output grows. At narrow
widths, copy wraps and secondary metadata compresses; the semantic order does not change.

The shell tail is intentionally quiet:

- one product/context row when useful;
- one attention row only when action is needed;
- composer label, buffer, and key hint;
- no repeated transcript summaries.

The status rows use one low-contrast surface and the focused composer uses a slightly lighter
surface inside a complete rounded frame. Both stay inside the existing tail height budget. The
luminance step and frame establish input ownership without adding another dashboard panel or
moving transcript history during resize.

## App-Owned Transcript Viewport

`TranscriptViewportUiState` owns the wrapped-row anchor, page height, maximum scroll, follow-tail,
latest and seen revisions, document identity, card hit areas, the last stable rendered-cell map,
and transcript selection.

- Default: follow the newest output.
- PageUp / mouse wheel up: freeze an absolute reader anchor.
- Streaming while frozen: keep the same visible rows and show `new output`.
- PageDown: approach the tail; reaching it resumes follow-tail.
- Ctrl+Home: jump to the first row.
- Ctrl+End: resume the newest row and clear unseen state.
- Thread/session switch: reset the viewport and stale card geometry.
- Left drag: freeze the current absolute reader anchor, highlight the selected rendered cells, and
  copy on release without waiting for another frame.
- Submitted user prompts, status rows, the composer, and transient badges are interaction chrome,
  not selectable transcript text. Assistant bodies and tool cards own independent semantic
  selection ranges, so a drag cannot leak across prompts or unrelated responses.
- A submitted prompt is one compact low-luminance row surface with a `›` intent marker and no
  separate `You:` role row. Explicit and soft-wrapped continuation rows retain the same surface;
  the marker appears only once.
- Mouse wheel input is accepted only inside the committed transcript rectangle. Scrolling over the
  composer or fixed shell tail does not move conversation history.
- Resize: invalidate selection geometry before the next pointer event; a stale frame receipt cannot
  restore it.

The terminal emulator does not own conversation history. No host scrollback delivery, cursor-based
history insertion, transcript handoff, or viewport replay mode exists in the production path.
Terminal-native text selection is still available as an explicit interaction mode: `:mouse off`
disables mouse reporting while retaining the same alternate-screen frame and `:mouse on` restores
app-owned pointer handling. Clipboard delivery is a thin terminal effect (OSC 52, including tmux
passthrough), never transcript authority.

Ready input batches coalesce consecutive drag coordinates to the newest point before the next
frame while preserving button-down, button-up, resize, and keyboard ordering. Crossterm all-motion
reports are discarded because Akra has no hover behavior; keeping them would place invisible work
ahead of the next real input. Frame admission is capped at a 16ms boundary, so a drag burst creates
at most one current frame per display interval rather than a delayed replay of old pointer samples.
`Ctrl+C` copies an active transcript selection before its interrupt/navigation/exit meanings are
considered.

Snapshot hydration projects an exact Akra main-session prompt envelope back to its user-authored
`user-prompt`. A hidden manual-intake handoff projects its `original-user-prompt`; execution,
reporting, task-authority, and rules sections never enter the visible canonical transcript. Text
that does not match the exact generated grammar is preserved unchanged.

## Tool and Diff Cards

Tool activity appears where it happened in the conversation.

- Collapsed read/explore: one meaningful label and a disclosure marker.
- Expanded read/explore: exact targets, paths, ranges, and available detail.
- Collapsed patch: affected file summary.
- Expanded patch: file header, hunk header, red removed rows, green added rows, neutral context.
- Click and Ctrl+E toggle the same stable digest, so keyboard and mouse cannot disagree.

Expansion is presentation state. It never mutates durable conversation content or creates a second
activity log. `:activity` remains an optional cross-turn inspector, not the only place tool detail
can be understood.

## Canonical Fullscreen Stream Surfaces

Parallel event rows remain in one app-owned viewport. No host scrollback delivery or durable/live
split is permitted.

Rendering uses explicit surfaces:

- `FullscreenTitledPanel` for bounded titled content;
- `FullscreenScrolledPanel` for selected or independently scrolled content;
- `FullscreenAppendOnlyStream` for ordered event rows.

`render_fullscreen_parallel_event_stream` is the only focused parallel stream renderer. Short
streams show a title. Under height pressure the title may hide, but the rows remain one stream and
their order is unchanged.

## Visual Grammar

- Cyan/teal: product identity and active affordance.
- White/default: primary prose.
- Muted gray: metadata and inactive hints.
- Amber: pending attention or degraded state.
- Red: failure and removed diff rows.
- Green: success and added diff rows.
- Magenta: sparing identity accents, never whole paragraphs.

Borders are reserved for focused overlays, cards, and the composer when they improve grouping.
The composer frame may be complete because it replaces, rather than adds to, the two chrome rows
already included in its height calculation.
Normal conversation prose remains borderless. The screen should read as a calm document, not a
dashboard made of boxes.

User prompt cards use fill, not another border. Their full-row background, one-cell marker inset,
and single blank message separator establish hierarchy while keeping more transcript rows available
than a role-label-plus-body block.

## Responsive Contract

- 80 columns: one column, compressed metadata, wrapped content, composer preserved.
- 120 columns: normal conversation density and full tool labels.
- 160 columns: more breathing room, never uncontrolled line length or extra authority panels.
- Short height: body yields first; the composer and required action row remain visible.
- Resize: wrapping and hit areas are recomputed in a new frame; the previous receipt is rejected if
  geometry changed during draw.

## Architectural Guardrails

1. Renderer code cannot read application services, Core authority, clocks, filesystem, network, or
   terminal I/O.
2. One terminal transaction captures one projection sample and one owned frame.
3. UI state commits only through a stable `FullscreenFrameRenderReceipt`.
4. Parallel geometry is finalized before rendering and never reconstructed from rendered text.
5. Alternate-screen, focus, mouse, bracketed-paste, cursor, and raw-mode restoration is best effort
   across every cleanup step.
6. New conversation features extend the canonical transcript/card model rather than adding a live
   buffer, host history, or separate visible log.
7. Selection reads only the rendered-cell snapshot returned by a stable frame receipt; controller
   input never guesses Ratatui wrapping from raw strings.
8. Composition owns the joinable terminal reader and its redacted panic boundary; the inbound TUI
   owns only ordered event reduction.
9. A frame burst is dirty-coalesced behind one 60Hz admission boundary. Terminal input continues to
   drain while the one admitted Ratatui transaction is in progress.

## Acceptance Scenarios

- A long conversation remains scrollable while the composer stays visible.
- Streaming append does not move a reader who paged up.
- The unseen badge appears only while output is newer than the reader's seen revision.
- Read/explore and patch cards expand by both click and Ctrl+E.
- Patch additions/removals render with semantic green/red backgrounds.
- Late assistant completion preserves interleaved tool ordering.
- A resize race does not apply stale scroll or hit-area state.
- Entering and exiting restores the caller's terminal cleanly on Windows and Linux families.
- Forward and reverse drags copy the same semantic order, preserve Korean/wide glyphs, and retain a
  visible selection background until the next selection or geometry invalidation.
- A high-rate drag settles to its latest coordinate without replaying stale highlight frames; plain
  pointer motion cannot delay the next key.
- Dragging a submitted prompt, status badge, or composer does not start transcript selection.
- `:mouse off` emits no mouse-reporting enable sequence at startup or after the mode change.
- A resumed Akra main session shows the original operator prompt, never its internal Codex prompt
  envelope.
