# TUI Shell Flow

This file defines operator-visible shell behavior on `prerelease`.

The canonical planning, directions, and supersession contract lives in
[../supersession/current-contract.md](../supersession/current-contract.md). This file focuses on
interaction flow and surface roles.

## Primary Loop

- The shell starts in a startup-aware conversation surface.
- The shell renders core app snapshots and sends user intent back as app commands where the core
  runtime owns the lifecycle.
- The operator can type before startup diagnostics finish.
- Once diagnostics allow submission, the active prompt is submitted and live stream output stays attached to the bottom of the shell.
- Completed assistant output moves into scrollback history while the live tail keeps the active prompt, transient stream text, and compact notices together.
- The operator alternates between conversation, inspection overlays, planning authoring, and queue/parallel execution without leaving the same terminal session.

## Current Shell Modes

| Mode | What the operator is doing | Entry | Exit |
| --- | --- | --- | --- |
| Conversation | typing prompts, watching stream output, reading compact status | default shell surface | submit a prompt or open an overlay |
| Diagnostics | checking startup readiness and failures | `Ctrl+d`, `:diag` | `Esc`, `Ctrl+c`, or toggle again |
| Sessions | searching and reopening previous threads | `Ctrl+o`, `:sessions` | open a session, start a draft, or close |
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
3. Core submits the turn effect and reduces stream completions into app state.
4. Live stream output stays in the inline tail until turn completion.
5. Tool activity, runtime notices, approval-review state, and warnings update the same shell surface.
6. When the turn completes, assistant output is committed into normal scrollback history.
7. Post-turn evaluation decides whether internal continuation advances, pauses, or stops.

## Planning And Continuation Flow

- Builtin `next-task` uses the current queue task derived from accepted planning.
- Detailed `:planning`, `:directions`, and lifecycle command semantics live in
  `docs/supersession/current-contract.md`.
- Accepted planning, the current queue task, proposed tasks, and repair status are reflected in the footer, queue overlay, and internal post-turn continuation.
- When the queue is actionable, continuation targets the current queue task.
- When the queue is valid but idle, behavior follows the queue-idle policy.

## Pause And Recovery States

- Startup blocked:
  the shell can render, but submit and session actions may remain gated.
- Planning invalid:
  post-turn continuation pauses until accepted planning validates again.
- Queue idle with `stop`:
  continuation ends after the current turn.
- Queue idle with the `review_and_enqueue` policy:
  a hidden planning worker may derive justified follow-up work.
- Repeated queue task:
  queue-driven continuation pauses until the planning queue advances beyond the previously handed-off task.
- Manual approval review:
  approval state is surfaced, but the shell still lacks interactive approve or deny actions.

## Code Entry

- Headless app runtime contracts live under `src/core`.
- Generic shell state reducers live under `src/adapter/inbound/tui/app`.
- Planning-specific TUI flow lives under `src/adapter/inbound/tui/app/planning`.
