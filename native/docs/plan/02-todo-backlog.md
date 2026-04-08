# TODO Backlog

This file now keeps the current open planning surface, not a long implementation backlog.

## Current Baseline
- live shell-first conversation flow
- startup diagnostics, recent sessions, and follow-up templates as inline shell inspections in main-buffer mode
- streamed turn updates, new-thread flow, and thread resume
- stable inline stream-history buffering with thread and turn lifecycle markers while live output stays separate until completion
- a transitional `Transcript / tail` viewport still exists in inline mode and should not be treated as the final flow target
- inline shell commands and lightweight transcript navigation
- builtin and workspace follow-up templates with reload, editable max turns, and stop rules
- shared adapter runtime reuse across startup, session, snapshot, turn, and GitHub polling paths
- session browser query, paging, recent-project filter, and result shaping
- approval, tool activity, warning, and GitHub review notices in shell status
- focused TUI module extraction and targeted shell/runtime tests

## Open Change Buckets
- terminal-flow reset
  - inline mode still presents `Transcript / tail` as a dedicated middle viewport, which keeps the shell too close to fullscreen frame thinking
  - the target is a Codex-CLI-like or Spring-Boot-like terminal flow with sequential top-to-bottom history and one tail prompt box
  - host terminal scrollback should become the primary history mechanism instead of the in-app transcript viewport
- platform validation
  - the validation matrix, capture helpers, checked-in record directory, summary helper, markdown report helper, and packaging docs are landed, but real macOS and Windows runs still need to be recorded
  - Windows-specific fixes should stay conditional on validated findings from the terminal-flow target instead of speculative portability edits
- maintenance
  - `app.rs` should stay near composition and shared-state ownership
  - runtime and UX changes should keep shipping with targeted regression coverage

## Document Rule
Only keep items here if they still describe the current state of open work across multiple PRs. Future feature-specific detail should move into separate feature docs.
