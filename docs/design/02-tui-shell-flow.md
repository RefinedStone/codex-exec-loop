# TUI Shell Flow

This file defines operator-visible shell behavior on `prerelease`. It describes the current flow, not the future redesign plan.

## Primary Loop

- The shell starts in a startup-aware conversation surface.
- The operator can type before startup diagnostics finish.
- Once diagnostics allow submission, the active prompt is submitted and live stream output stays attached to the bottom of the shell.
- Completed assistant output moves into scrollback history while the live tail keeps the active prompt, transient stream text, and compact notices together.
- The operator alternates between conversation, inspection overlays, planning authoring, and post-turn automation without leaving the same terminal session.

## Current Shell Modes

| Mode | What the operator is doing | Entry | Exit |
| --- | --- | --- | --- |
| Conversation | typing prompts, watching stream output, reading compact status | default shell surface | submit a prompt or open an overlay |
| Diagnostics | checking startup readiness and failures | `Ctrl+d`, `:diag` | `Esc`, `Ctrl+c`, or toggle again |
| Sessions | searching and reopening previous threads | `Ctrl+o`, `:sessions` | open a session, start a draft, or close |
| Automation | editing post-turn automation policy and preview | `Ctrl+f`, `:auto` | close overlay |
| Queue Inspection | reading the current queue task, proposed tasks, and skip summary | `:queue`, `:q` | close overlay |
| Planning Controls | staging or reopening planning workspace flows | `:planning` | close overlay or enter editor/review flow |
| Directions Maintenance | editing supporting direction and queue-idle artifacts | `:directions` | close overlay or enter staged editor flow |

## Startup And Session Flow

- Startup diagnostics begin immediately on launch.
- The shell can render before diagnostics finish, but submit only proceeds when diagnostics allow it.
- Queued manual input auto-submits once startup becomes ready.
- `Ctrl+o` opens recent sessions and loads a selected snapshot back into the main shell.
- `Ctrl+t` returns to a blank draft.
- When startup is blocked, diagnostics remain the source of truth for why the shell cannot proceed.

## Conversation Turn Flow

1. The operator types in the main composer or enters a `:` command.
2. `Enter` submits when startup and runtime state allow it.
3. Live stream output stays in the inline tail until turn completion.
4. Tool activity, runtime notices, approval-review state, and warnings update the same shell surface.
5. When the turn completes, assistant output is committed into normal scrollback history.
6. Post-turn evaluation decides whether automation continues, pauses, or stops.

## Planning And Automation Flow

- Automation controls own stop rules, max-turn policy, preview rendering, and planner debug visibility.
- Builtin `next-task` uses the current queue task derived from accepted planning.
- `:planning` and `:directions` both route through staged planning drafts and explicit promotion.
- Accepted planning, the current queue task, proposed tasks, and repair status are reflected in the footer, automation overlay, queue overlay, and post-turn automation.
- When the queue is actionable, automation targets the current queue task.
- When the queue is valid but idle, behavior follows the queue-idle policy.

## Pause And Recovery States

- Startup blocked:
  the shell can render, but submit and session actions may remain gated.
- Planning invalid:
  post-turn automation pauses until accepted planning validates again.
- Queue idle with `stop`:
  automation ends after the current turn.
- Queue idle with the `review_and_enqueue` policy:
  a hidden planning worker may derive justified follow-up work.
- Repeated queue task:
  queue-driven automation pauses until the planning queue advances beyond the previously handed-off task.
- Manual approval review:
  approval state is surfaced, but the shell still lacks interactive approve or deny actions.

## Code Entry

- Generic shell state reducers live under `src/adapter/inbound/tui/app`.
- Planning-specific TUI flow lives under `src/adapter/inbound/tui/app/planning`.
