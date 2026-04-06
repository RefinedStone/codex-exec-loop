# Current Product State

## What The Latest Branch Already Does
The `prerelease` branch is no longer just a dashboard prototype. It now supports:

- startup checks and account diagnostics
- recent session browsing from `thread/list`
- thread history loading from `thread/read`
- new thread start and existing thread resume
- prompt submission through `turn/start`
- streamed agent deltas and completed items rendered in the shell
- builtin auto follow-up strategies
- workspace follow-up templates loaded from `.codex-exec-loop/followups/`
- auto-stop rules for `AUTO_STOP` and no-file-change turns

## Why The UX Still Feels Different From Codex CLI
Even with live shell behavior, the app still feels more page-based than Codex CLI because:

- startup, session list, and shell are separate screens
- the shell is entered through navigation, not as the default home
- each major action still opens a fresh app-server connection
- there is no long-lived runtime that keeps one shell session attached to one transport process

## Current Strengths
- the shell already renders real transcript updates
- auto follow-up is visible and controllable from the UI
- the codebase still follows a clear hexagonal split
- the app-server protocol work is kept behind one outbound adapter

## Immediate Documentation Goal
All docs should assume the current branch already has streaming shell behavior and auto follow-up. Future planning should build on that baseline instead of describing the older placeholder shell.

