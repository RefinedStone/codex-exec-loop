# Auto Follow-Up And Templates

## Current Capability
The `prerelease` branch already includes a meaningful auto follow-up loop. This is now one of the native client's core differentiators and should be treated as a first-class feature in future design work.

## Builtin Template Strategies
The app currently exposes builtin template variants for:

- next-task
- plan-queue
- bugfix
- docs

These templates are rendered with runtime values such as:

- `{auto_turn}`
- `{max_auto_turns}`
- `{session_id}`
- `{stop_keyword}`
- `{last_message}`

## Workspace Templates
Workspace templates are loaded from:

- `.codex-exec-loop/followups/*.md`
- `.codex-exec-loop/followups/*.txt`

The current adapter sorts files, ignores unsupported extensions, and records warnings for empty templates.

## Stop Rules
The current shell can stop auto follow-up when:

- the agent emits the configured stop keyword, default `AUTO_STOP`
- the "no file changes" rule is enabled and the last completed turn changed nothing

The activity panel now keeps the last auto follow-up skip reason visible so the operator can tell why the loop did not continue.

## Current UI Controls
Inside the shell:

- `Ctrl+a`: toggle auto follow-up
- `Ctrl+f`: cycle templates
- `Ctrl+p`: open the template preview overlay
- `:templates`: open the template preview overlay from the composer
- `Ctrl+g`: edit the stop keyword value
- `Ctrl+k`: toggle stop keyword rule
- `Ctrl+n`: toggle no-file-change stop rule

Inside the template preview overlay:

- `Up/Down` or `j/k`: move between templates
- `Ctrl+g`: enter stop keyword edit mode
- while editing, type the new keyword directly, `Enter` to save, `Esc` or `Ctrl+c` to cancel
- `PageUp/PageDown` or `Ctrl+u/Ctrl+d`: scroll long previews
- `Enter`, `Esc`, or `Ctrl+c`: close the overlay

## Remaining Gaps
- stop keyword editing still assumes a single token and only supports append/backspace editing
- template preview is read-only and still cycles through a flat list
- no richer strategy metadata beyond label and source
