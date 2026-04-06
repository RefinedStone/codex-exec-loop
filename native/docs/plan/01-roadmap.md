# Roadmap

## R1. Make The Existing Shell Feel Primary
- reduce dependence on full-screen navigation
- let users reach recent sessions and startup status without leaving the shell mindset
- keep diagnostics available, but stop treating them as the main product surface

## R2. Stabilize Runtime Lifecycle
- move from action-scoped app-server processes toward a longer-lived runtime model
- keep current event mapping and domain contracts intact
- improve reconnect, reset, and failure recovery behavior

## R3. Improve Shell Ergonomics
- better multiline input editing and focus behavior
- clearer shell status and transport activity
- richer template inspection and clearer stop-rule visibility

## R4. Strengthen Auto Follow-Up
- keep the current builtin and workspace-template support intact
- add safer controls around stop conditions and operator visibility
- preserve file-change-based stopping and stop-keyword behavior

## R5. Split TUI Responsibilities
- keep `app.rs` readable
- extract focused shell state and reducer logic when new complexity is added
- avoid moving protocol or persistence code into the TUI layer
