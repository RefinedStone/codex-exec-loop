# TUI Layered Architecture And Aesthetic Contract

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
| Controllers and effects | Service calls, command dispatch, runtime side effects | `shell_controller.rs`, `app_runtime.rs`, planning controllers | Status copy, panel titles, layout dimensions |
| Projection and copy | View models, `Line` content, labels, status wording, key footer text | `shell_presentation.rs`, `shell_presentation/**`, `planning/presentation.rs` | `Frame`, `Layout`, terminal side effects, raw color decisions |
| Theme and chrome | Semantic styles, Akra brand tokens, panel frame helpers, selection markers | `theme.rs` | Feature state, controller behavior, surface-specific wording |
| Rendering and layout | `Rect`, `Layout`, `Frame`, `Paragraph`, `List`, popup and inline section placement | `shell_rendering/**`, `inline_layout.rs`, `popup_frame.rs`, `popup_helpers.rs` | New keybinding claims, new product copy, raw color or border policy |
| Terminal adapters | Crossterm/Ratatui lifecycle, scrollback, viewport replay, host terminal side effects | `ratatui_frontend.rs`, `inline_terminal_adapter.rs`, `history_insertion.rs` | Planning semantics, Akra copy, overlay policy |
| Tests and captures | Rendering contracts, snapshot deltas, terminal validation evidence | `shell_rendering_tests.rs`, `shell_rendering_contract_tests.rs`, `snapshots/**`, `scripts/capture_native_validation.*` | Unreviewed visual contract drift |

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

- The inline tail must remain borderless and compact.
- It must keep a stable hierarchy: status ribbon, planning or queue summary, runtime notice,
  prompt, command hint.
- It must reserve display density for long-running operator work, not marketing copy.
- It must support Korean and wide-character prompt text without changing the surrounding layout
  contract.

### Popup And Inspection Overlays

- Popup overlays must use the shared Akra panel frame from `AkraTheme::panel_block`.
- Overlay content should follow this order when the surface needs all sections: header, summary,
  primary content, status, keys.
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
4. Keep new modules under roughly the same context budget rules as `docs/agent/01-project-playbook.md`.
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
