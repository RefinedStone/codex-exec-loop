# Current Product and Operator Contract

[한국어](../ko/reference/current-product.md)

This document is the canonical shipped-behavior reference for `prerelease`. Future work belongs in
an explicitly proposed document, not here.

## Product Shape

- Akra is a native-first Rust client over official `codex app-server` interfaces.
- The inline main-buffer TUI is the primary surface; completed output is committed to host terminal
  scrollback while the live viewport owns the prompt, stream tail, overlays, and compact notices.
- `src/core` coordinates headless app commands, effects, completions, events, and snapshots.
- CLI, Admin, Telegram, and automation adapters reuse application services instead of owning
  separate planning or parallel policy.

## Operator Surfaces

| Surface | Entry | Shipped behavior |
| --- | --- | --- |
| Conversation | default | draft prompts, stream a turn, review approvals, and continue work |
| Diagnostics | `Ctrl+d`, `:diag` | inspect startup readiness and blockers |
| Sessions | `Ctrl+o`, `:sessions` | search, rename, resume, or start a blank draft |
| Reviews | `:reviews` | inspect the bounded review center projection |
| Activity | `:activity [all\|diff\|output\|command\|patch\|…]`, `:act` | inspect retained progressive activity cards; expand selected detail (live/overlay only; host scrollback stays static) |
| Queue | `:queue`, `:q`, `akra queue` | inspect accepted head, proposals, skip framing, and receipts |
| Planning | `:planning`, `:planning-init` | stage, validate, and promote planning changes |
| Directions | `:directions` | maintain directions and queue-idle supporting artifacts |
| Health | `:doctor`, `:planning doctor`, `akra doctor`, `akra status` | inspect planning authority without authoring |
| Parallel | `:parallel`, `:pa` | enable or refresh automation and open the supervisor board |
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
:planning [doctor]
:doctor
:reset <queue|directions|all>
:new
:help
```

`:turns` controls single-session auto-follow. Parallel automation is a separate explicit opt-in.
`:stop` terminates active app-server sessions, closes the active parallel epoch, and disarms both
continuation paths. A later `:parallel` re-arms only parallel continuation.

## Turn and Approval Flow

1. Input may be drafted before startup completes; submission waits until readiness permits it.
2. Core issues the turn effect and reduces typed app-server stream events into app state.
3. Active output remains in the live inline tail; final assistant output moves to committed history.
4. Typed activity, runtime notices, approvals, and warnings update the same shell projection.
5. Post-turn evaluation advances, pauses, or stops continuation from accepted planning state.

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
- Workers run unattended with `workspace-write`, leave edits uncommitted, and decline approvals.
  The host validates the exact lease, worktree, branch, frozen base, changed-file bounds, and final
  cleanliness before creating a source commit.
- Delivery is serial: source push, PR automation/review verification, exact reviewed-range
  integration into the configured branch, remote verification, and identity-checked cleanup.
- Remote, repository, visibility, integration branch, base OID, source SHA, and reviewed commit
  range are frozen before delivery. Drift blocks instead of falling back to mutable checkout state.
- Human review is required by default. Public-repository or autonomous delivery requires an exact
  parent-process opt-in; repository configuration cannot grant either permission.

The board is a read-only projection of readiness, pool slots, active roster, selected lifecycle,
distributor head, queue state, and withheld-dispatch reason.

## Recovery and Limits

- Invalid or conflicting planning updates pause continuation and preserve review evidence.
- Store-backed claims allow official refresh and distributor recovery after restart; recovery does
  not hard-reset operator-owned integration history.
- Non-Git workspaces use planning authority but not the full Git worktree pool.
- Planning detail authoring is manual; the `llm-assisted` editor path is disabled.
- Real-terminal evidence is still required for primitive-sensitive TUI changes and affected
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
