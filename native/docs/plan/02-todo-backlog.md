# TODO Backlog

This file now keeps the current open planning surface, not a long implementation backlog.

## Current Baseline
- live shell-first conversation flow
- startup diagnostics, recent sessions, and follow-up templates as inline shell inspections in main-buffer mode
- streamed turn updates, new-thread flow, and thread resume
- inline shell commands and lightweight transcript navigation
- builtin and workspace follow-up templates with reload, editable max turns, and stop rules
- shared adapter runtime reuse across startup, session, snapshot, turn, and GitHub polling paths
- session browser query, paging, recent-project filter, and result shaping
- approval, tool activity, warning, and GitHub review notices in shell status
- focused TUI module extraction and targeted shell/runtime tests

## Open Change Buckets
- shell ergonomics
  - streamed output still needs a scrollback-safe history shape instead of frame-style replay assumptions
- platform validation
  - the validation matrix and packaging docs are landed, but real macOS and Windows runs still need to be recorded
  - Windows-specific fixes should stay conditional on validated findings instead of speculative portability edits
- migration docs
  - the repository root README still carries more Python/legacy weight than the current native-first product story
- maintenance
  - `app.rs` should stay near composition and shared-state ownership
  - runtime and UX changes should keep shipping with targeted regression coverage

## Document Rule
Only keep items here if they still describe the current state of open work across multiple PRs. Future feature-specific detail should move into separate feature docs.
