# Inline Scrollback Shell

This document is the detailed feature plan for the large refactor that moves the native client from the current redraw-heavy main-buffer TUI toward a Codex-CLI-like inline shell.

This file is intentionally detailed. The compactness rule applies to completed baseline docs and posture docs, not to a major active refactor where detailed planning reduces execution risk.

## 1. Why This Work Exists

The current shell is usable, but `MainScreen` is still not a true scrollback-friendly terminal mode.

Observed problems:
- startup can visually overlap with prior terminal content in some terminal programs
- scrolling the terminal can make the shell feel like it is being redrawn as one moving screen instead of leaving stable history behind
- modal overlays and fixed panel assumptions still make the shell feel like a fullscreen TUI even when alternate-screen is off

This is a product problem, not just a cosmetic problem. The current behavior weakens the "CLI shell" feel and makes long-running usage less trustworthy across terminals.

## 2. Product Target

The target is not a Claude-Code-style append-only REPL.

The target is a Codex-CLI-like inline TUI:
- scrollback should remain meaningful conversation history
- the shell can still use raw mode
- the shell can still keep a live interaction region near the terminal tail
- the shell should preserve the current conversation-shell mental model where practical
- alternate-screen fullscreen mode should remain available as fallback during migration

This means the refactor is not "remove TUI." It is "stop treating the main buffer as a fullscreen frame."

## 3. Current Diagnosis

### 3.1 Runtime Diagnosis

`src/adapter/inbound/tui/app/app_runtime.rs` currently does all of the following in one runtime path:
- creates the terminal backend
- enables raw mode unconditionally
- conditionally enters alternate-screen only when the environment flag is enabled
- still calls `terminal.draw(...)` on every loop iteration
- owns input polling, background message handling, effect execution, and rendering together

This means `MainScreen` is only "alternate-screen off." It is not architecturally different from the fullscreen renderer.

### 3.2 Rendering Diagnosis

`src/adapter/inbound/tui/app/shell_rendering.rs` currently assumes one composited frame:
- `draw_conversation_shell` clears the whole area with `Clear`
- layout is rebuilt as header + transcript + footer + composer
- overlays are rendered as modal layers on top of that frame
- transcript scrolling is handled as an in-frame viewport, not terminal scrollback

This is the exact shape that causes the main-buffer mode to feel like an inline fullscreen app rather than a flow-oriented shell.

### 3.3 Presentation Diagnosis

`src/adapter/inbound/tui/app/shell_presentation.rs` is already partly reusable, but still speaks in fullscreen-shell terms:
- transcript title assumes in-frame paging
- status title assumes control legends for overlay-heavy interaction
- input title assumes a dedicated composer block
- status summaries are good reusable content, but not yet neutral enough for multiple frontends

### 3.4 State Diagnosis

The state split is better than the old monolith and is worth preserving:
- `conversation_input.rs`
- `conversation_intents.rs`
- `conversation_lifecycle.rs`
- `conversation_runtime.rs`
- `followup_controls.rs`
- `followup_overlay_ui.rs`
- `session_overlay_ui.rs`
- `shell_controller.rs`
- `transcript_viewport.rs`

The problem is not missing state decomposition. The problem is that the runtime and renderer still interpret that state as one fullscreen composition.

## 4. Refactor Boundary

### 4.1 Areas To Keep Stable

These areas should remain mostly intact:
- `src/domain/*`
- `src/application/port/*`
- `src/application/service/*`
- `src/adapter/outbound/*`
- stream event reduction inside `conversation_runtime.rs`
- lifecycle and intent reduction already extracted under `src/adapter/inbound/tui/app/*`

Reason:
- the problem is terminal rendering behavior, not app-server protocol coverage
- the current hexagonal split is already useful for this migration
- rewriting service/domain code would add risk without fixing the terminal issue

### 4.2 Areas To Refactor Heavily

These areas are the real migration surface:
- `src/adapter/inbound/tui/app/app_runtime.rs`
- `src/adapter/inbound/tui/app/shell_rendering.rs`
- `src/adapter/inbound/tui/app/shell_presentation.rs`
- `src/adapter/inbound/tui/app/shell_controller.rs`
- `src/adapter/inbound/tui/app/transcript_viewport.rs`
- `src/adapter/inbound/tui/app/followup_overlay_ui.rs`
- `src/adapter/inbound/tui/app/session_overlay_ui.rs`

Reason:
- these files encode the fullscreen assumptions
- these files decide how transcript, overlays, input, and viewport are presented
- these files are the right place to split inline mode from alternate-screen mode

## 5. Non-Goals

This work should not do the following:
- do not turn the product into a plain line-oriented REPL
- do not remove multiline input just because fullscreen composition is going away
- do not remove startup diagnostics, recent sessions, auto follow-up, template browsing, stop rules, or warnings
- do not rewrite outbound adapters unless the inline-shell contract exposes a real boundary problem
- do not collapse the reducer split back into one runtime object
- do not delete alternate-screen mode before the inline path is proven across terminals

## 6. Success Contract

The new main-buffer shell is successful only if all of the following are true.

### 6.1 Scrollback Contract

- once transcript-worthy output reaches the terminal history, the shell should not repaint it as part of a whole-frame redraw
- terminal scrollback should read like one coherent session
- scrolling the terminal should not feel like the application itself is being replayed downward

### 6.2 Interaction Contract

- the shell should still feel like one active conversation surface
- input should remain available near the tail of the terminal
- multiline entry should remain supported if technically practical
- current shortcut habits should be preserved where they do not force the fullscreen model back in

### 6.3 Capability Contract

The migration is incomplete if any of these regress:
- startup diagnostics and gating
- recent-session loading and selection
- new-thread flow
- resume-thread flow
- snapshot loading
- streamed delta handling
- streamed completion handling
- tool activity visibility
- warning visibility
- auto follow-up control
- template visibility
- stop keyword control
- no-file-change stop control
- skip-reason visibility

## 7. Target Architecture

## 7.1 High-Level Split

The shell should become three layers:

1. Shared shell runtime
2. Frontend-neutral presentation helpers
3. Multiple frontends

The runtime should own behavior.
The presentation layer should own human-readable summaries.
The frontend should own terminal-specific rendering and input interaction.

## 7.2 Shared Shell Runtime

The new runtime layer should own:
- startup checks
- recent-session loading
- conversation snapshot loading
- turn submission
- background message polling
- auto follow-up scheduling
- reducer dispatch and effect execution
- state transitions that are independent of ratatui

The runtime layer should not own:
- `Frame` drawing
- bordered layout
- popup geometry
- transcript viewport layout math
- inline vs fullscreen rendering choice

This is the most important extraction because both frontends should share the same behavior engine.

## 7.3 Frontend-Neutral Presentation

`shell_presentation.rs` should move toward reusable text summaries.

Keep here:
- status labels
- input hints
- warning summaries
- thread metadata summaries
- auto follow-up summaries
- transcript line formatting helpers where still useful

Move out of it:
- block titles that only make sense with bordered panels
- pager legends that only make sense with in-frame scrolling
- popup-first key legends
- assumptions that input always lives in a boxed composer

## 7.4 Dual Frontend Model

The product should explicitly support two frontends during migration.

### Inline Frontend

Purpose:
- default main-buffer experience
- scrollback-friendly rendering
- no whole-frame redraw of the terminal history region

Expected traits:
- stable transcript output
- compact live region near terminal tail
- inline inspections instead of fullscreen popups

### Alternate Frontend

Purpose:
- preserve the current fullscreen ratatui behavior as fallback
- reduce rollout risk while inline mode matures

Expected traits:
- keep current box layout and overlays as long as needed
- act as compatibility path, not product target

## 8. Rendering Model

## 8.1 Current Rendering Model To Replace

Current fullscreen assumptions:
- one root frame
- one transcript viewport inside that frame
- one footer panel
- one composer panel
- overlays on top

This model is acceptable only for alternate-screen fallback.

## 8.2 Inline Rendering Model

The inline frontend should conceptually split rendering into:

1. Stable transcript history
2. Live interaction region

### Stable Transcript History

This should contain anything that deserves durable history:
- thread-open markers
- user prompts after submission
- agent output as it becomes stable enough to append
- tool activity
- warnings that matter historically
- auto follow-up decisions
- session switch markers
- important diagnostics transitions

### Live Interaction Region

This should contain the temporary surface near the terminal tail:
- current startup/session/turn state summary
- current prompt buffer
- inline inspection content when diagnostics/session/template mode is active
- short control hints relevant to the current state

The live region may redraw in place. The stable history region should not be treated as a redraw surface.

## 8.3 Streaming Output Strategy

Streaming is the hardest part and must be defined explicitly.

V1 direction:
- preserve the current runtime semantics around "one active turn at a time"
- append meaningful stream output into transcript history
- allow the live region to continue showing input status while streaming
- allow prompt buffering during streaming
- keep submit blocked until the current turn completes
- surface when buffered manual input exists so auto follow-up behavior remains understandable

Do not attempt to solve advanced concurrent editing semantics before the basic scrollback contract is stable.

## 9. Input Model

## 9.1 What Must Be Preserved If Practical

- multiline input
- `Ctrl+j` or a close equivalent for newline insertion
- inline commands such as diagnostics/session/template access
- buffered prompt behavior while a turn is still active

## 9.2 What Must Change

- input can no longer assume a permanent fullscreen composer block
- control legends must be shorter and state-aware
- session/template/diagnostics flows cannot require popup ownership in main-buffer mode
- transcript navigation should stop being the primary way to read prior conversation in inline mode because terminal scrollback becomes the primary history mechanism

## 9.3 Explicit Decision

Do not simplify the shell into a plain one-line prompt just to make rendering easier.

The goal is not "shell minimalism." The goal is "inline terminal behavior without whole-frame redraw."

## 10. Overlay Strategy

The current overlay concept should be split by frontend.

### 10.1 Alternate Frontend

Alternate-screen may continue using overlays during migration.

### 10.2 Inline Frontend

Inline mode should reinterpret overlays as inline inspections or command-driven flows.

Diagnostics:
- show current diagnostics summary in the live region
- print important readiness changes to transcript history when useful

Sessions:
- show session list and selected session detail in the live region
- print a thread-switch marker after opening a session

Templates:
- show template selection, source, preview summary, and stop-rule state in the live region
- keep stop-keyword editing without requiring a popup

Exit confirmation:
- keep it explicit, but allow a compact inline confirmation state rather than a modal dialog

## 11. State Migration Guidance

Not every current UI state object should survive unchanged.

### 11.1 `transcript_viewport.rs`

Expected change:
- reduce its importance for inline mode
- keep it mainly for alternate-screen fallback or for any limited live-region scrolling that remains useful

Reason:
- terminal scrollback replaces most transcript paging responsibility

### 11.2 `session_overlay_ui.rs`

Expected change:
- keep selection-state logic if useful
- remove the assumption that selection exists only inside a popup list

### 11.3 `followup_overlay_ui.rs`

Expected change:
- preserve stop-keyword editing state
- reconsider preview scroll as popup-specific state
- reinterpret list/preview behavior for an inline inspection surface

### 11.4 `shell_controller.rs`

Expected change:
- stop treating diagnostics/sessions/templates primarily as overlay toggles
- make command-driven and inline-inspection flows first-class

### 11.5 Presentation Mode Naming

Current naming:
- `MainScreen`
- `AlternateScreen`

Target naming direction:
- explicit inline main-buffer shell
- explicit alternate-screen shell

Reason:
- current naming hides the real architectural problem by making main-buffer mode sound more distinct than it actually is

## 12. Proposed Implementation Sequence

## Phase 0: Lock Target And Vocabulary

Deliverables:
- keep this plan as the source of truth for the workstream
- align terminology around "inline shell" vs "alternate-screen shell"
- stop describing the target as a fully append-only REPL

Exit criteria:
- docs and implementation discussion use the same target vocabulary

## Phase 1: Extract Shared Runtime

Primary goal:
- separate behavior orchestration from ratatui frame rendering

Concrete tasks:
- isolate event polling, background-message consumption, reducer dispatch, and effect execution from the current draw loop
- define a controller/runtime object that can drive more than one frontend
- keep behavior identical while the alternate-screen renderer still owns display

Files most affected:
- `src/adapter/inbound/tui/app/app_runtime.rs`
- possibly new runtime/controller files under `src/adapter/inbound/tui/app/`

Exit criteria:
- alternate-screen path still works
- shared shell behavior can be driven without directly coupling to `Terminal.draw(...)`

## Phase 2: Split Frontends

Primary goal:
- make inline and alternate frontends explicit

Concrete tasks:
- keep the existing fullscreen renderer as alternate-screen fallback
- introduce a dedicated inline frontend path behind a flag
- make renderer selection an explicit branch in the runtime instead of a minor alternate-screen toggle

Files most affected:
- `src/adapter/inbound/tui/app/app_runtime.rs`
- `src/adapter/inbound/tui/app/shell_rendering.rs`
- any new inline renderer modules

Exit criteria:
- both frontends can start and share the same behavior engine

## Phase 3: Inline MVP

Primary goal:
- achieve a minimally useful scrollback-friendly main-buffer shell

Concrete tasks:
- print stable transcript history without whole-frame redraw
- keep a live region for current input and status
- support startup state, prompt submission, streaming updates, and thread switching
- keep diagnostics/sessions/templates minimally usable

Files most affected:
- inline renderer module(s)
- `shell_presentation.rs`
- `shell_controller.rs`

Exit criteria:
- one can run a normal conversation in inline mode without the current redraw artifact

## Phase 4: Parity And Interaction Hardening

Primary goal:
- close the feature and ergonomics gap between inline mode and the current shell

Concrete tasks:
- improve inline diagnostics/session/template inspection
- carry over stop-rule editing cleanly
- refine input hints and shortcut behavior
- reduce leftover fullscreen vocabulary in presentation and state

Files most affected:
- `shell_presentation.rs`
- `followup_overlay_ui.rs`
- `session_overlay_ui.rs`
- `shell_controller.rs`

Exit criteria:
- inline mode preserves the capability floor listed above

## Phase 5: Validation And Default Switch

Primary goal:
- make inline mode the default main-buffer experience

Concrete tasks:
- validate behavior across representative terminals
- confirm fallback alternate-screen path still works
- remove only the old main-buffer fullscreen assumptions, not the fallback renderer itself

Exit criteria:
- inline mode is stable enough to become default

## 13. Testing Strategy

## 13.1 Automated Tests To Preserve

Keep and extend tests around:
- reducer behavior
- startup gating
- session loading
- snapshot loading
- stream reduction
- auto follow-up decisions
- stop-rule behavior

These tests protect the product logic that should survive the UI refactor.

## 13.2 New Automated Tests To Add

### Shared Runtime Tests

Validate:
- background message handling order
- state transitions independent of frontend
- effect execution behavior across startup, resume, submit, stream, and completion paths

### Presentation Tests

Validate:
- inline summaries and hints remain coherent
- warnings, skip reasons, and thread markers are formatted predictably
- presentation helpers no longer assume fullscreen-only layout

### Inline Frontend Tests

Validate:
- transcript-worthy events are appended in the right order
- live region updates do not require full-frame redraw assumptions
- key interaction states remain visible during streaming and gating conditions

## 13.3 Manual Validation Matrix

Minimum terminal set:
- iTerm2
- Terminal.app
- Ghostty
- WezTerm

Optional but valuable:
- tmux
- zellij

Checklist:
- no startup overlap with prior terminal content
- no "UI replaying downward" feeling when scrolling
- usable long-session scrollback
- stable streaming behavior
- usable session switch flow
- usable template and stop-rule interaction
- fallback alternate-screen mode still works

## 14. Rollout Rules

- inline mode should start behind a flag
- alternate-screen remains fallback until inline mode is proven
- default should switch only after parity and terminal validation are both acceptable
- do not delete fallback support just because inline mode becomes default

## 15. Main Risks

### 15.1 Streaming Plus Input Editing

This remains the hardest part.

Risk:
- maintaining editable input near the terminal tail while output is arriving is terminal-sensitive

Response:
- keep turn-submission semantics conservative in v1
- avoid speculative concurrency behavior changes during the rendering migration

### 15.2 Popup-State Leakage

Risk:
- overlay-first state and language can keep forcing fullscreen assumptions back into inline mode

Response:
- explicitly reinterpret overlay state in inline mode
- treat alternate-screen as the only place where popup behavior remains natural

### 15.3 Scope Creep

Risk:
- the migration can accidentally expand into unrelated service or domain rewrites

Response:
- hold the refactor boundary around inbound runtime/presentation first

## 16. Acceptance Criteria

This workstream is ready for default inline mode only when all of the following are true:
- main-buffer mode no longer redraws the whole terminal frame
- scrollback reads as one continuous conversation history
- the capability floor is preserved
- representative terminals behave acceptably
- alternate-screen fallback remains available

## 17. Decision Rule

When old fullscreen behavior conflicts with the new main-buffer contract:
- prefer the inline-shell contract for main-buffer mode
- preserve fullscreen behavior only in alternate-screen fallback

If a design choice does not improve scrollback behavior, reduce redraw risk, or preserve a key existing capability, it should not widen the scope of this refactor.
