# Roadmap

## R1. Adopt A Stream-First Shell UX
- treat the conversation as one vertical transcript instead of a dashboard-style layout
- keep the prompt composer anchored at the bottom while the transcript flows above it
- prefer terminal scrollback and append-only output over full-screen page transitions

## R2. Stabilize Runtime Lifecycle
- move from action-scoped app-server processes toward a longer-lived runtime model
- keep current event mapping and domain contracts intact
- improve reconnect, reset, and failure recovery behavior

## R3. Reduce Full-Screen TUI Surface Area
- shrink the role of alternate-screen panels and persistent sidebars
- move diagnostics, session browse, and template inspection toward lighter overlays or command-style entry points
- ensure the shell can stay primary even when auxiliary information is needed

## R4. Strengthen Auto Follow-Up
- keep the current builtin and workspace-template support intact
- add safer controls around stop conditions and operator visibility
- preserve file-change-based stopping and stop-keyword behavior

## R5. Re-scope Inbound UI Responsibilities
- keep `app.rs` readable while the shell model changes
- extract focused shell state and reducer logic when the stream-first UX settles
- avoid moving protocol or persistence code into the UI layer
