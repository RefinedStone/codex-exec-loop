# Auto Follow-Up And Templates

Auto follow-up is now part of the native client's core product behavior and should keep more context than the UI-only docs.

## Template Sources
- builtin strategies: `next-task`, `plan-queue`, `bugfix`, `docs`
- workspace templates: `.codex-exec-loop/followups/*.md` and `.codex-exec-loop/followups/*.txt`
- workspace loading stays sorted, ignores unsupported extensions, and records warnings for empty templates

## Runtime Values
Templates can render runtime placeholders such as:

- `{auto_turn}`
- `{max_auto_turns}`
- `{session_id}`
- `{stop_keyword}`
- `{last_message}`

## Stop Rules
The current shell can stop auto follow-up when:

- the agent emits the configured stop keyword, default `AUTO_STOP`
- the no-file-change rule is enabled and the last completed turn produced no file changes

Stop-keyword matching is case-insensitive and token-based, so surrounding punctuation does not block a match.

The latest skip reason should remain operator-visible after a turn finishes.

## Operator Controls
- `Ctrl+a`: toggle auto follow-up
- `Ctrl+f`: cycle templates forward
- `Ctrl+p` or `:templates`: open template preview
- `Ctrl+g`: enter stop-keyword edit mode, opening the template overlay if needed
- `Ctrl+k`: toggle stop-keyword rule
- `Ctrl+n`: toggle no-file-change rule

## Durable Behavior To Preserve
- template selection and stop settings belong to shell state, not to ad hoc render logic
- follow-up decisions happen after completed turn reduction, not before the turn result is understood
- stop-keyword values are constrained to non-empty identifier-like tokens using letters, numbers, or underscores
- workspace templates extend builtin behavior; they should not replace builtin availability
- future UX work can change how controls are presented, but should not hide why auto follow-up did not continue
