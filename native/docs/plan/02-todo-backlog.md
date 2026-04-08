# TODO Backlog

This file now keeps the current open planning surface, not a long implementation backlog.

## Current Baseline
- live shell-first conversation flow
- startup diagnostics and recent sessions as shell-adjacent overlays
- streamed turn updates, new-thread flow, and thread resume
- inline shell commands and lightweight transcript navigation
- builtin and workspace follow-up templates with stop rules
- shared adapter runtime reuse across startup, session, snapshot, and turn paths
- focused TUI module extraction and targeted shell/runtime tests

## Open Change Buckets
- runtime continuity
  - fallback behavior during concurrent work is still part of the current runtime story
  - reconnect, reset, and warning handling still need to stay visible and predictable
- shell ergonomics
  - overlays and raw-mode shell chrome are still transitional
  - input editing and long-session handling are still weaker than the target shell feel
- automation ergonomics
  - auto follow-up must stay understandable and operator-visible while the shell changes
  - new controls should preserve the current stop-rule model unless that model is explicitly redesigned
- maintenance
  - `app.rs` should stay near composition and shared-state ownership
  - runtime and UX changes should keep shipping with targeted regression coverage

## Document Rule
Only keep items here if they still describe the current state of open work across multiple PRs. Future feature-specific detail should move into separate feature docs.
