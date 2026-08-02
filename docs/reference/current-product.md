# Current Product and Operator Contract

[한국어](../ko/reference/current-product.md)

This document is the canonical shipped-behavior reference for `prerelease`. Future work belongs in
an explicitly proposed document, not here.

## Product Shape

- Akra is a native-first Rust client over official `codex app-server` interfaces.
- The alternate-screen fullscreen TUI is the primary surface. One app-owned transcript viewport
  owns conversation history, streaming rows, tool cards, overlays, status, and the composer.
- Losing the controlling terminal terminates the native TUI through its normal shutdown boundary;
  a closed terminal or PTY must not leave the TUI or its app-server runtime resident.
- Agent text stays raw Markdown in app-server state and is rendered at the TUI projection boundary;
  fenced-code delimiters and language labels are not shown as conversation text.
- `src/core` coordinates headless app commands, effects, completions, events, and snapshots;
  composition-owned `NativeClientRuntime` gives the TUI one typed `CoreInput` dispatch boundary.
- CLI, Admin, Telegram, and automation adapters reuse application services instead of owning
  separate planning or parallel policy.

## Operator Surfaces

| Surface | Entry | Shipped behavior |
| --- | --- | --- |
| Conversation | default | draft prompts, stream a turn, review approvals, and continue work |
| Diagnostics | `Ctrl+d`, `:diag` | inspect startup readiness and blockers |
| Sessions | `Ctrl+o`, `:sessions` | search, rename, resume, or start a blank draft |
| Reviews | `:reviews` | inspect the bounded review center projection |
| Activity | `:activity [all\|diff\|output\|command\|patch\|…]`, `:act` | inspect a chronological typed activity timeline with outcome, exact elapsed time, foldable retained detail, and line-numbered unified diff; read/explore and patch detail is also expandable in the conversation |
| Queue | `:queue`, `:q`, `akra queue` | inspect accepted head, proposals, skip framing, and receipts |
| Planning | `:planning`, `:planning-init` | stage, validate, and promote planning changes |
| Directions | `:directions` | maintain directions and queue-idle supporting artifacts |
| Health | `:doctor`, `:planning doctor`, `akra doctor`, `akra status` | inspect planning authority without authoring |
| Parallel | `:parallel`, `:pa` | enable or refresh automation and open the focused Parallel Operations board |
| Parallel peek | `:peek` | inspect active parallel agent conversations |

Global keys include `Ctrl+q` to exit, `Ctrl+t` to start a blank draft, and `Esc`/`Ctrl+c` to close
the active inspection surface. A modal may override global keys while it owns focus; visible key
copy must match the current controller path.

## Shell Command Registry

The source registry is `src/adapter/inbound/tui/app/inline_shell_commands.rs`.

```text
:diag
:parallel [off]
:peek
:activity [all|diff|output|command|patch|mcp|plan|reason|agent|terminal|token|guardian|moderation|unknown]
:sessions
:reviews
:queue
:directions
:turns <positive|infinite|off>
:stop
:model [default]
:view [simple|medium|detail]
:language [english|korean]
:think <none|minimal|low|medium|high|xhigh|default>
:copy [selection|last]
:mouse [on|off|toggle]
:planning [doctor]
:doctor
:reset <queue|directions|all>
:new
:help
```

`:turns` controls single-session auto-follow. Parallel automation is a separate explicit opt-in.
`:stop` terminates active app-server sessions, closes the active parallel epoch, and disarms both
continuation paths. A later `:parallel` re-arms only parallel continuation.

In the conversation, a left-button drag selects the exact rendered transcript cells, keeps their
background highlighted, and copies the resulting text through OSC 52 when the button is released.
The clipboard path supports direct terminals and tmux passthrough. `:copy selection` repeats the
last completed selection; `:copy last` copies the latest raw assistant answer. `:mouse off` hands
drag selection back to the terminal emulator without leaving fullscreen, while `:mouse on`
restores app-owned scrolling, cards, and selection. `Shift+drag` remains the terminal-native bypass
on emulators that provide it. Set `AKRA_TUI_MOUSE_CAPTURE=off` before startup to begin in the
terminal-native mode.

## Turn and Approval Flow

1. Input may be drafted before startup completes; submission waits until readiness permits it.
2. Core admits one turn submission at a time; an accepted dispatch issues exactly one worker effect
   and permits the TUI to clear the editor and append the prompt to transcript history.
3. `Tab` can confirm delivery into the exact active turn. Core admits one correlated steer worker,
   keeps the draft until provider acknowledgement, and ignores stale completions; later edits or
   identical retyped input are never cleared by an older acknowledgement.
4. Streaming assistant output updates its canonical `item_id` row in place. Tool rows append at
   arrival time, so a late final assistant event cannot reorder the transcript.
5. Typed activity, runtime notices, approvals, warnings, and expandable tool cards update the same
   fullscreen projection.
6. Post-turn evaluation advances, pauses, or stops continuation from accepted planning state.

The Activity surface joins progressive payloads with authoritative item-lifecycle records by exact
item identity. It exposes active, completed, failed, declined, interrupted, and observed outcomes
without deriving progress from prose. Exact active waits distinguish model response, subagent,
approval, task output, and retry. `Up`/`Down` selects a row; `Enter`, `e`, or a left click folds or
expands retained detail. App-server `Read`, `ListFiles`, and `Search` command actions project a
concise typed target header and reveal exact bounded targets only when expanded; raw command text is
not retained for these typed actions, and reads with no output still appear from lifecycle truth.
Completed rows stay visually quiet while active work remains dominant. Unified diffs show old/new
source line numbers and extend addition/deletion backgrounds across the code region while keeping
the gutter neutral.

Only the interactive main conversation can answer a reviewable command or bounded additional
permission request. `Y` accepts once; `N`/`Esc` declines; `Enter` is inert. Timeout, interrupt,
disconnect, invalid payloads, uninspectable file-change requests, and unattended workers fail
closed. No session-wide grant is cached.

## Planning Contract

- Planning follows `draft -> validate -> promote`.
- SQLite behind `PlanningTaskRepositoryPort` is the accepted task, direction, queue, claim, and
  runtime authority.
- `.codex-exec-loop/planning/` holds operator-authored details, prompts, staged drafts, exports, and
  rejected-write evidence. It is not the task-authority database.
- Builtin `next-task` and internal continuation execute only the accepted queue head.
- Proposed tasks remain visible but non-executable until accepted.
- Queue-idle behavior comes from accepted direction authority.
- Admin/API intake creates one validated `ready` task and never interrupts an `in_progress` task.
- `akra planning-tool` is the structured list/create/update boundary for automation callers.
- Hidden planning workers are read-only. The host validates and applies their structured commands.

Authority lives below `${AKRA_HOME:-~/.akra}/projects/<project>/runtime/planning-authority.db` for
both Git and non-Git workspaces. Git repositories use a private incarnation marker in the canonical
Git common directory so linked worktrees share one authority and independent clones do not.

## Parallel Contract

- Bare `:parallel`/`:pa` is enable-or-refresh; `:parallel on` is not implemented.
- The first off-to-on entry checks readiness, reconciles the guarded three-slot pool, opens an
  automation epoch, and dispatches accepted ready work up to idle capacity.
- Re-entering while enabled refreshes readiness and projection; it does not reset the pool or open
  a second epoch.
- `:parallel off` stops local automation and invalidates late dispatch results but leaves worktrees
  for guarded recovery. Closing the board does not disable parallel mode.
- Pool, lease, task, session, and distributor mutations are serialized through application and
  durable cross-process gates. TUI state never decides capacity, retry, or dispatch policy.
- The application control-plane's bounded unsettled-cleanup ledger is the sole cleanup-notice
  authority. The TUI captures its typed owned projection into the conversation screen model and
  renders it without a separate adapter or conversation ledger.
- Global cleanup-notice events invalidate the current frame for redraw only; exact retry settlement
  changes the control-plane projection.
- Workers run unattended with `workspace-write`, leave edits uncommitted, and decline approvals.
  The host validates the exact lease, worktree, branch, frozen base, changed-file bounds, and final
  cleanliness before creating a source commit.
- Delivery is serial: source push, PR automation/review verification, exact reviewed-range
  integration into the configured branch, remote verification, and identity-checked cleanup.
- Remote, repository, visibility, integration branch, base OID, source SHA, and reviewed commit
  range are frozen before delivery. Drift blocks instead of falling back to mutable checkout state.
- Human review is required by default. Public-repository or autonomous delivery requires an exact
  parent-process opt-in; repository configuration cannot grant either permission.

The focused Parallel Operations board uses the same alternate-screen fullscreen frame transaction.
It presents one priority blocker, accepted queue pressure, a lifecycle timeline,
three slot lanes, and a selected-lane delivery checklist. Pool slots, active roster, bounded session
detail, optional role profile, distributor head, queue state, and withheld-dispatch reason remain a
read-only projection; a missing join is shown as `DESYNC` or `unknown`, never invented progress.
Accepted dispatch work is visually separate from active leases.

Selection is keyed by slot/session identity and survives projection reorder on refresh. `Enter` or
`Space` inspects the selected lane, `V` opens the read-only active-agent picker, `Ctrl+R` refreshes,
`Ctrl+P` disables parallel mode, and `Esc` closes the focused board. Closing restores the unchanged
composer while the enabled passive projection continues; submitting a parallel task reopens the
board so dispatch progress is visible. The 80-column layout stacks lanes before selected detail,
while 120- and 160-column layouts show lifecycle, lanes, and detail together.

Parallel event delivery keeps one generation-qualified canonical event window. The frame capture
turns it into one app-owned `ParallelLiveStreamModel`, finalizes geometry before drawing, and never
asks the renderer to rediscover ownership from text. Under height pressure the stream title may
collapse, but event order and rows remain in the same Ratatui viewport. There is no host frontier,
history insertion, durable/live split, or replay mode.

`:peek` opens a read-only active-agent conversation preview; switching agents or overlays prevents
late results from replacing the latest preview or the interactive conversation.

The local Admin graphic dashboard keeps those facts passive, but its harness controls are real
application commands. CSRF-protected browser actions can enable the automation epoch, request the
next accepted task, refresh the supervisor projection, or disable the loop through one persistent
`ParallelModeControlPlaneHandle`. The HTTP adapter drains typed background completions back into
that handle before reporting control state; it does not call Git, GitHub, pool, or distributor
adapters directly.

The `/admin/akra` game projection uses a PixiJS 8 world over the same dashboard snapshot. Its
validated frontend store, semantic camera zoom, worker movement, furniture occlusion, and scene
selection remain presentation-only; see [Admin Game Frontend](admin-game-frontend.md).

For UI/UX debugging, `akra admin --debug-harness` starts an application-owned, non-durable Fake
scenario clock behind the same dashboard contract. It supports normal delivery, blocked recovery,
and queue-pressure playback without mutating real planning, parallel, Git, or GitHub authority.

## Recovery and Limits

- Invalid or conflicting planning updates pause continuation and preserve review evidence.
- Store-backed claims allow official refresh and distributor recovery after restart; recovery does
  not hard-reset operator-owned integration history.
- Non-Git workspaces use planning authority but not the full Git worktree pool.
- Planning detail authoring is manual; the `llm-assisted` editor path is disabled.
- Real-terminal evidence is still required for primitive-sensitive fullscreen TUI changes and affected
  restart/distributor/multi-worktree flows.

## Code Entry

- Core: `src/core/`
- TUI: `src/adapter/inbound/tui/`
- CLI: `src/adapter/inbound/cli.rs`
- Admin: `src/adapter/inbound/admin_api/`
- Telegram: `src/adapter/inbound/telegram_bot/`
- Planning: `src/application/service/planning/`, `src/domain/planning/`
- Parallel: `src/application/service/parallel_mode/`, `src/domain/parallel_mode/`
- App-server adapter: `src/adapter/outbound/app_server/`
- SQLite authority: `src/adapter/outbound/db/sqlite_planning_authority_adapter.rs`
