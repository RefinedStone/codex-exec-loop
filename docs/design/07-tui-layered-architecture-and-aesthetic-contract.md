# TUI Layered Architecture And Aesthetic Contract

[한국어](../ko/reference/tui-contract.md)

## Context And Goals

The native shell TUI must stay easy to edit in small context windows. A change to wording should
not require reading terminal adapter code, a change to layout should not invent product copy, and a
change to color should not scatter Ratatui primitives through feature modules.

This contract defines where each TUI concern lives and which visual rules are non-negotiable for
the fixed Akra theme.

## Layer Stack

| Layer | Owns | Primary files | Must not own |
| --- | --- | --- | --- |
| State and reducers | User intent, mode transitions, selected indices, editing state | `shell_chrome.rs`, `conversation_*`, `*_ui_state.rs`, planning state modules | Ratatui widgets, operator-facing visual hierarchy, raw styles |
| Controllers and effects | Service calls, command dispatch, runtime side effects | `shell_controller.rs`, `queue_overlay_controller.rs`, `app_runtime.rs`, planning controllers | Status copy, panel titles, layout dimensions |
| Projection and copy | View models, `Line` content, labels, status wording, key footer text | `shell_presentation.rs`, `shell_presentation/**`, `planning/presentation.rs` | `Frame`, `Layout`, terminal side effects, raw color decisions |
| Frame capture and delivery receipt | One owned `InlineShellFrameModel`, active `InlineInspectionFrameModel`, and compare-and-apply render feedback | `inline_frame_model.rs`, `inline_terminal_adapter.rs` | Service calls, provider I/O, draw-time authority reads, optimistic state mutation |
| Theme and chrome | Semantic styles, Akra brand tokens, panel frame helpers, selection markers | `theme.rs` | Feature state, controller behavior, surface-specific wording |
| Rendering and layout | Owned frame models, `Rect`, `Layout`, `Frame`, `Paragraph`, `List`, popup and inline section placement | `shell_rendering/**`, `inline_layout.rs`, `popup_frame.rs`, `popup_helpers.rs` | `NativeTuiApp`, Core/application/control-plane authority, state mutation, new keybinding claims, product copy, raw color or border policy |
| Terminal adapters | Crossterm/Ratatui lifecycle, typed parallel delivery, scrollback, viewport replay, host terminal side effects | `ratatui_frontend.rs`, `inline_terminal_adapter.rs`, `parallel_terminal_delivery.rs`, `history_insertion.rs` | Planning semantics, Akra copy, overlay policy |
| Tests and captures | Rendering contracts, snapshot deltas, terminal validation evidence | `shell_rendering_tests.rs`, `shell_rendering_contract_tests.rs`, `snapshots/**`, `scripts/capture_native_validation.*` | Unreviewed visual contract drift |

`NativeTuiApp` is not a general-purpose bag passed between these layers. It owns exactly four
private typed slices (`shell`, `conversation`, `planning`, `runtime`). Production frontend and
terminal modules receive owned projections/models through `ShellRuntime`; only test fixtures may
request the aggregate directly. Planning-worker display state is a sealed Core projection, so
navigation intent cannot optimistically rewrite it.

## Where To Edit

| Need | Start here | Then verify |
| --- | --- | --- |
| Change wording, labels, prompt tail text, status lines, or key footer text | Projection and copy layer | Focused unit test or snapshot for the affected surface |
| Change whether an action is available | State or controller layer before copy | Reducer/controller test plus visible key/footer assertion |
| Add or rename an overlay section | Projection view model first, rendering second | Overlay contract test and snapshot |
| Change border, selection, title, key, success, warning, danger, muted, or accent styling | `theme.rs` only | `bash scripts/check_tui_layering.sh` and focused rendering test |
| Change popup or inline geometry | Rendering and layout layer | Snapshot plus narrow and wide viewport capture when practical |
| Change scrollback, resize, alt-screen, or inline replay behavior | Terminal adapter layer | Follow `docs/validation/terminal-ui-testing-methodology.md` |
| Add a keybinding | Input reducer/controller first | Display it only after the input path exists and is tested |

## Design Tokens And Foundations

- The first theme is fixed as `Akra`; runtime theme switching is not part of the baseline contract.
- `AkraTheme` must be the semantic token source for brand, accent, success, warning, danger, muted,
  subtle, shortcut, selected, panel, title, key, and list marker treatment.
- Raw `Color::*`, raw `.bg(...)`, raw `Block::default().borders(...)`, and raw list highlight
  symbols must stay out of feature modules. Add a semantic helper to `theme.rs` when the existing
  helper vocabulary is insufficient.
- ANSI status colors may remain semantic inside `AkraTheme`; product modules must call semantic
  helpers rather than choosing terminal colors directly.
- The shell must preserve readable contrast without relying on whole-terminal background control.

## Component-Level Rules

### Inline Shell Tail

- The inline tail must remain compact. Status, notices, and inspection summaries stay
  borderless; the focused prompt composer may own one semantic focus frame, provided
  the frame does not claim the whole terminal background or alter host scrollback.
- In the main-buffer frontend that frame is an open `╭ / │ / ╰` focus rail, not a
  full-width horizontal box. This avoids resize reflow moving the cursor or live tail
  outside the physical viewport.
- It must keep a stable hierarchy: status ribbon, planning or queue summary, runtime notice,
  prompt, command hint.
- One terminal sync transaction must call `revisioned_planning_parallel_projection()` exactly once
  and own the returned `RevisionedPlanningParallelProjection` together with the render-clock values
  in `ConversationProjectionSample`. This narrow Core projection carries one revision plus the
  planning/parallel state required by the frame; it must not clone the `AppSnapshot` startup,
  session-catalog, or conversation payloads. The pre-history outer flow layout and the final
  tail/live/cache projection consume that same sample. UI-local facts may be reread after a handoff
  acknowledgement.
- Host-scrollback delivery must bind its conversation diff baseline to the semantic history-identity
  revision captured in that sample. A new draft or an accepted Ready snapshot for a different
  session resets only the conversation baseline; selection intent, deferred/failed loads,
  draft-to-provider promotion, and same-thread reattach preserve it. Transient Loading/Failed
  projections must not impersonate an authoritative empty transcript, and conversation switches
  must not reset the parallel event baseline or shared physical-row geometry.
- Parallel host delivery uses the stable stream generation/ordinal, never rendered text, as its
  side-effect identity. One adapter-owned planner partitions the canonical event window into
  disjoint durable and live models; only an exact committed host receipt advances the monotonic
  frontier, independently of frame feedback and conversation handoff.
- The newest undelivered parallel event remains live even when its wrapped rows exceed the event
  viewport. Focused Parallel Operations anchors the hidden physical cursor at the live viewport
  origin, and every newly durable host batch carries the shared physical reflow guards so later
  resize cleanup cannot erase the boundary event or move panel chrome into host history.
- A released transcript handoff must carry one typed delivery token sampled with the frame. The
  token binds conversation history identity, thread and source-turn identity, handoff generation,
  and the exact transcript revision. Host-scrollback and parallel writes place that token in the
  successful `HistoryFlushResult`; viewport replay carries it through the successfully drawn frame.
  Only a token that still matches the current handoff may acknowledge delivery. A stale receipt,
  unrelated conversation switch, or later transcript append must leave the current handoff pending.
- A failed terminal draw must not acknowledge its handoff or leave the staged frame signature
  trusted. The next transaction must repaint the unchanged semantic frame before it can emit an ACK.
- `NativeTuiApp` owns one opaque `NativeClientRuntime`, not a parallel control-plane handle, sink,
  or completion channel. Core and parallel intents enter through `NativeClientEvent`; the runtime
  fairly drains its private completion lanes and returns typed outcomes for adapter projection.
  Every parallel effect must settle success, failure, or panic exactly once before its in-flight
  correlation can be released.
- Supersession is covered by that consistency guarantee: the sample owns one control-plane
  presentation projection, one event-stream projection, and the narrow owned Core projection,
  while row planning and drawing consume the same owned overlay view. High-frequency prompt,
  pulse, and scheduler checks share a panel-only sample so they do not clone transcript or
  event-stream rows.
- One `ConversationScreenModel` derived from that sample must own the local UI facts for a frame.
  Tail copy, live transcript copy, cursor layout, and frame-cache comparison consume that same
  immutable projection; presentation helpers must not reread `NativeTuiApp`, call services, or
  sample their own clocks.
- Before `Terminal::draw`, the terminal transaction must materialize one owned
  `InlineShellFrameModel`. Its `InlineInspectionFrameModel` variant owns every active overlay's
  view, local widget state, geometry-dependent scroll decision, and expected feedback baseline.
  Frame capture may sample UI-local state but must not reacquire Core, application services, the
  parallel control plane, or provider I/O.
- Production `shell_rendering.rs` and `shell_rendering/**` functions consume that owned model.
  They may mutate only Ratatui's `Frame`; they must not accept or reread `NativeTuiApp`, dispatch
  commands, access services or clocks, or retain mutable adapter state. The pure draw returns one
  `InlineFrameRenderReceipt` instead.
- The terminal transaction may apply that receipt only after draw, cursor and terminal-size reads,
  and the post-draw resize snapshot all succeed. An exact render-attempt gate rejects failed,
  resize-raced, stale, and duplicate receipts; applying a receipt uses compare-and-apply baselines
  so a newer UI edit cannot be overwritten by an older frame.
- Prompt focus has one policy shared by input and presentation. Exit and turn-steer dialogs remove
  prompt focus and hide the terminal cursor; closing either dialog restores the unchanged draft and
  its exact cursor position. Focused Supersession owns the inline inspection viewport, hides the
  cursor, and consumes unhandled text so it cannot mutate the preserved composer. Closing it while
  parallel mode remains enabled returns to the passive projection and restores composer focus.
- It must reserve display density for long-running operator work, not marketing copy.
- It must support Korean and wide-character prompt text without changing the surrounding layout
  contract.
- Row-limited tail compaction must consume semantic priority assigned with each projected line; it
  must not infer priority by parsing localized rendered copy. Queue mutation pending, refresh, and
  undo truth remains pinned across language and modal row budgets, while planning marks only
  projected rows containing an actionable blocker as warnings.
- GitHub review setup must remain `PendingFirstFrame` until a draw and its post-draw size
  verification both succeed. A failed or resize-raced draw dispatches no setup; the first stable
  delivery dispatches exactly one `AppCommand::SetupGithubReviewPolling`. Core owns setup
  generation/workspace admission and polling cursor/single-flight rules. Composition owns Git,
  credential, discovery, service construction, and the exact-correlation service registry. Polling
  ticks dispatch `AppCommand::PollGithubReview`; the TUI owns only environment parsing, poll timing,
  and setup/poll status projection.
- `:stop` and running-turn Ctrl-C must pause local automation first, then dispatch
  `AppCommand::RequestStopAllSessions`. Core owns stop generation, active-submission correlation,
  single-flight admission, the continuation pause/rearm transition, and the one `TurnStarted`
  synchronization attempt. `RequestStopAllSessions` alone must fail closed even when another
  inbound adapter does not run TUI cleanup commands. The TUI must not keep a second
  pending-interrupt boolean or call the provider directly. A lifecycle transition may invalidate
  the worker permit, but new turn admission and newly requested conversation loading remain behind
  the physical stop lease until the exact worker completion settles.
- Cancelled manual-prompt preparation keeps its physical Core lease until exact worker settlement.
  Its worker must recheck cancellation after blocking reads and before bootstrap or task-authority
  mutations; stale completion filtering alone is not a side-effect guard.

### Conversation Markdown And Diff Detail

- Activity rows are chronological typed lifecycle views. Each visible row keeps one kind, outcome,
  bounded summary, optional exact elapsed time, and a fold affordance; prose must not be parsed to
  invent state or progress.
- Completed activity is visually quiet, current activity is dominant, and exact waits distinguish
  model response, subagent, approval, task output, and retry. When the Activity overlay already
  owns the exact wait line, the conversation tail must not duplicate a coarser working line.
- Keyboard and mouse interaction address the same card identity and bounded expansion state. Hit
  areas are frame receipts and must be discarded after a resize or stale draw.
- App-server agent text is raw Markdown. The transcript projection must interpret its presentation
  syntax consistently for live deltas, completed history, and viewport replay.
- Fenced code delimiters and their info strings are parser syntax, not transcript content. Hide
  matching opening and closing fences, retain incomplete streaming code, and style only the code
  body through `AkraTheme`.
- The Activity Diff document must interpret unified-diff file headers and hunk ranges. Each code row
  uses one right-aligned line-number gutter: deletions show the old line number, while additions and
  context show the new line number.
- Diff additions, deletions, gutters, metadata, and hunk separators use semantic theme styles.
  Wrapped continuation rows keep an empty gutter and sign column instead of repeating a source line
  number.
- Diff pagination owns a semantic parser cursor with old/new counters and wrapped-line continuation
  state. Previous and next pages must restore that cursor instead of rescanning the retained
  document or guessing line numbers from rendered text.
- Malformed, binary, combined, or retention-truncated diff fragments remain safely visible as muted
  metadata; control characters must never reach the terminal as executable escape sequences.

### Append-only Stream Surfaces

- A stream surface is not a titled panel once its rows can be split across durable host scrollback
  and the live inline viewport.
- No panel title may be inserted between durable scrollback rows and live rows. If the whole stream
  fits, the renderer may show the normal section title; if the stream overflows, the live tail must
  be titleless live tail data only.
- Inline inspection rendering must choose an explicit render surface type: `InlineTitledPanel` for
  ordinary titled sections, `InlineScrolledPanel` for ordinary scrolled sections, and
  `InlineAppendOnlyStream` for rows that can be split across host scrollback and the live viewport.
- Parallel event stream rendering must enter through the named stream renderer and
  `InlineAppendOnlyStream` instead of calling a generic titled section helper with ad hoc title copy.
- Any change to stream row retention, scroll offset, title visibility, or live-tail chrome must add
  a frame-recorder regression for the exact before/after redraw sequence.

### Popup And Inspection Overlays

- Popup overlays must use the shared Akra panel frame from `AkraTheme::panel_block`.
- Overlay content should follow this order when the surface needs all sections: header, summary,
  primary content, status, keys.
- Focused Supersession must remain inside the main-buffer model and use the complete 16-row live
  viewport; it must not enter alternate-screen mode or move its panel chrome into host scrollback.
- Parallel Operations must join pool, roster, bounded session detail, and distributor rows by exact
  identity. It must render disagreement as `DESYNC`, missing facts as `unknown`, and accepted queue
  work separately from active leases. Presentation must not infer gate completion from prose.
- Lane selection must survive refresh reorder by slot/session identity. Wide layouts show
  lifecycle, lanes, and selected detail in columns; narrow layouts show every lane before selected
  detail. Both retain refresh, disable, inspect, agent-view, and close controls.
- Every actual overlay identity change must leave the shell reducer as one typed
  `ShellOverlayTransition { from, to, exit_mode }`. The root coordinator has one exhaustive cleanup
  owner for all overlay variants and both exit modes; explicit close only dispatches
  `OverlayClosed`. Entering `Approval` is `Suspend` and preserves the departed overlay's local
  state; `DirectionsMaintenance` is the one overlay restored after approval closes. Normal exits
  clean the departed overlay exactly once.
- Approval decisions must enter through `AppCommand::SubmitApprovalDecision`. Core owns matching
  the current pending approval by full approval/server-request identity and the
  submitting/submitted single-flight state; composition performs the provider call. A duplicate
  exact request preserves that lease, while a stale same-label resolution cannot close a newer
  modal. The TUI owns only modal projection and retry status copy.
- The Review Center read-only overlay controller must dispatch a core load command. Core owns
  latest-wins correlation and completion; composition executes the application authority reads
  into a core-owned snapshot.
- Review Center projection and rendering must read only that screen model; displaying its loading
  state, resizing, or repeatedly redrawing it must not perform service, repository, filesystem, or
  database I/O.
- One pre-draw frame capture must create an owned `SessionOverlayScreenModel` containing catalog
  status, workspace context, committed and edited query state, filter/page projection, stable
  selected thread identity, page-local selected index, rename state, and warning/key availability.
  Filtering, paging, and selection repair run exactly once while capturing that model;
  presentation helpers and renderers must not reread `NativeTuiApp` or call services.
- Session list rows, selected detail, warnings, and key copy must be built from that same model.
  Capture must finish the owned `SessionOverlayView` and a frame-local Ratatui `ListState` before
  drawing. Only the stable `InlineFrameRenderReceipt` may compare-and-apply that list state, and
  resize or repeated redraw must not trigger session-catalog I/O.
- Core owns session-catalog admission and correlation. Overlay open/startup sends ensure-loaded
  intent and explicit reload sends refresh intent; the adapter must not prewrite `Loading`, inspect
  its `SessionState` projection to suppress either intent, or create a second coalescing rule.
- Core coalesces one exact in-flight workspace/limit and publishes accepted loading transitions.
  The adapter-local `SessionState` is presentation-only. A Core-accepted rename projection cannot
  be vetoed by a missing or newer local editor receipt; that receipt settles only presentation
  draft, feedback, selection, and status.
- Async Review Center results may replace the current screen model only when the core correlation
  generation, workspace, and active-thread identity (thread ID) still match; workspace or thread
  identity drift must trigger a correlated reload.
- Opening the Queue overlay must dispatch its chrome before dispatching a core authority-load
  command. Core owns latest-wins correlation and completion; composition executes the coherent
  application read. The terminal input path must not read planning services, repositories,
  filesystems, or databases.
- Queue projection and rendering must consume a core-correlated immutable screen model. Loading and
  failed authority states are read-only and must not advertise remove or undo shortcuts.
- A ready Queue screen model must bind its visible runtime rows, selection, planning revision, and
  destructive-action tokens to the same authority snapshot. Correlation generation, workspace,
  active-thread, or visible planning-revision drift must invalidate that snapshot and trigger a
  correlated reload.
- Queue remove and undo actions must enter through a core command. Core owns the monotonic
  generation and single-flight gate; the TUI may expose that accepted generation as pending status
  but must not mint its own operation ID.
- The queue mutation gate must outlive Queue overlay chrome. Closing the overlay may reset local
  selection, feedback, and hit areas, but must not discard an in-flight mutation or apply an
  uncorrelated completion.
- Queue rows and undo receipts must not change optimistically. Both successful and failed mutation
  results require an authoritative refresh before the exact operation and workspace/thread context
  may settle visible state.
- `PlanningQueueUseCases` must own the cancellation transaction: submit the cancellation, then
  perform coherent runtime and queue-authority readback after either success or failure.
  Composition runs that transaction as a core effect and maps its completion into core-owned DTOs.
  The TUI Queue controller owns only remove/undo presentation and projects settlement from the
  accepted core correlation; it must not schedule the worker or compose cancellation and readback
  service calls.
- Selected rows must use `AkraTheme::selected()` and `AkraTheme::list_highlight_symbol()` or the
  option-line helpers that wrap them.
- Key footers must use `AkraTheme::key_line`.
- Key footers must only name shortcuts that the current state actually handles.

### Titles, Brand, And Masthead

- Titled shell surfaces must use an Akra title treatment, usually through `AkraTheme::title_line`.
- The brand may appear as `Akra / <surface> / <mode>` or in the startup masthead.
- Startup masthead height must stay bounded so it never hides the conversation input area.

### Warning And Error States

- Warning and error copy must be explicit about the operator impact.
- Warning and error color must come from `AkraTheme::warning()` and `AkraTheme::danger()`.
- Recovery keys must be shown only when implemented in the current state.

## Accessibility Requirements

- Every interactive surface must be usable from the keyboard path already owned by the controller
  or reducer.
- Selection must be visible through both marker and style, not color alone.
- Long lines should wrap or be clipped by the existing surface contract instead of expanding layout
  unpredictably.
- Snapshot or terminal capture validation should include at least one narrow terminal when changing
  layout or dense copy.

## Content And Tone Standards

- TUI copy should sound like an operations tool: concise, literal, and state-aware.
- Prefer concrete action labels such as `rerun checks`, `open queue`, or `close` over vague labels
  such as `go`, `continue`, or `manage` when the target action is known.
- Do not describe non-existent future commands, theme toggles, or shortcuts.
- Do not add explanatory onboarding prose to every surface; keep help text local to help and setup
  flows.

## Anti-Patterns

- Do not start a visual change in rendering when the real change is wording or state projection.
- Do not add `Color::Cyan`, `Color::Green`, `Color::Black`, `.bg(...)`, `Block::default()` panel
  frames, or raw `highlight_symbol("> ")` outside the theme/chrome layer.
- Do not duplicate key footer strings across rendering files.
- Do not make one overlay visually special unless the underlying interaction requires a distinct
  component contract.
- Do not update snapshots until the source change has been reviewed as an intentional visual
  contract change.

## LLM Editing Guardrails

1. Open this file and the smallest file in the layer table that matches the requested change.
2. Decide the layer before editing. If the request is about copy, do not edit rendering first.
3. Use `AkraTheme` for all visual styling and markers.
4. Keep new modules under the context budget rules in `docs/reference/development.md`.
5. Run `bash scripts/check_tui_layering.sh` before PR review for TUI visual or presentation work.
6. Add or update the smallest focused test or snapshot that proves the visible contract.

## QA Checklist

- The change is in the layer that owns the concern.
- No new raw Ratatui chrome primitive escaped `theme.rs` or an approved terminal adapter exception.
- Displayed shortcuts correspond to implemented input paths.
- Selection remains visible through marker and style.
- Korean or wide-character text still fits the affected surface when the change touches prompt,
  tail, or overlay copy.
- Snapshot changes are intentional and named in the PR body.

## Related Design Decisions

- [Typed Terminal Delivery Transaction](08-typed-terminal-delivery-transaction.md) is accepted for
  implementation but does not describe shipped behavior until its completion criteria pass.
