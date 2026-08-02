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
- Resize: invalidate selection geometry before the next pointer event; a stale frame receipt cannot
  restore it.

The terminal emulator does not own conversation history. No host scrollback delivery, cursor-based
history insertion, transcript handoff, or viewport replay mode exists in the production path.
Terminal-native text selection is still available as an explicit interaction mode: `:mouse off`
disables mouse reporting while retaining the same alternate-screen frame and `:mouse on` restores
app-owned pointer handling. Clipboard delivery is a thin terminal effect (OSC 52, including tmux
passthrough), never transcript authority.

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
Normal conversation prose remains borderless. The screen should read as a calm document, not a
dashboard made of boxes.

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
- `:mouse off` emits no mouse-reporting enable sequence at startup or after the mode change.
