# Auto Follow-Up And Templates

Auto follow-up is already part of the shipped client.

## Template Catalog

- builtin templates: `builtin next-task`, `builtin plan-queue`, `builtin bugfix`, `builtin docs`
- workspace templates: `.codex-exec-loop/followups/*.md` and `.codex-exec-loop/followups/*.txt`
- workspace loading stays sorted, ignores empty templates, and records load warnings instead of failing the whole catalog

## Runtime Placeholders

Templates can render:

- `{auto_turn}`
- `{max_auto_turns}`
- `{session_id}`
- `{stop_keyword}`
- `{last_message}`

## Current Runtime Rules

- stop when the agent emits the configured stop keyword; default is `AUTO_STOP`
- stop when the no-file-change rule is enabled and the completed turn produced no file changes
- stop-keyword matching is case-insensitive and token-based
- the latest skip or block reason stays operator-visible after the turn

## Planning-Aware Behavior

- invalid planning files block auto follow-up for every template
- non-planning templates can still run when planning is uninitialized
- builtin `next-task` requires planning state:
  - uninitialized: blocked because there is no actionable queue context
  - ready with task: runs against the current queue head
  - ready with no task + `queue_idle.policy = stop`: stops automation explicitly
  - ready with no task + `queue_idle.policy = review_and_enqueue`: runs a hidden queue-manager review before deciding whether another turn should be queued

## Operator Controls

- `Ctrl+a`: toggle post-turn automation
- `:stop`: turn post-turn automation off explicitly
- `Ctrl+f`: cycle templates
- `Ctrl+p` or `:templates`: open template preview
- `Ctrl+g`: edit stop keyword
- `Ctrl+k`: toggle stop-keyword rule
- `Ctrl+n`: toggle no-file-change rule
- `Ctrl+l`: edit max auto turns

## Durable Contract

- template selection and stop settings belong to shell state
- follow-up decisions run after completed turn reduction, not before
- stop keywords stay constrained to non-empty identifier-like tokens
- workspace templates extend builtin availability; they do not replace it
