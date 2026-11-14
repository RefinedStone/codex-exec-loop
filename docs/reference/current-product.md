# Current Product and Operator Contract

[한국어](../ko/reference/current-product.md)

This document is the canonical shipped-behavior reference for `prerelease`. Future work belongs in
an explicitly proposed document, not here.

## Product Shape

- Akra is a native-first Rust client over official `codex app-server` interfaces.
- The alternate-screen fullscreen TUI is the primary surface. One app-owned transcript viewport
  owns conversation history, streaming rows, tool cards, overlays, status, and the composer.
- The startup logo occupies that transcript viewport; startup, draft editing, and active turns use
  one bottom-anchored status/composer geometry, so the first submission never moves the input box.
- Losing the controlling terminal terminates the native TUI through its normal shutdown boundary;
  a closed terminal or PTY must not leave the TUI or its app-server runtime resident.
- Agent text stays raw Markdown in app-server state and is rendered at the TUI projection boundary;
  fenced-code delimiters and language labels are not shown as conversation text.
- Submitted operator prompts render as compact full-width low-luminance cards with one `›` marker.
  They do not spend a separate row on a `You:` label, keep the same surface through wrapping, and
  remain non-selectable interaction chrome. A non-default origin such as `Auto Follow-up` remains
  visible as a compact inline badge inside the card.
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
| Configuration | `akra config` | inspect effective values and their origins; safely edit global or Git-worktree TOML settings |
| Parallel | `:parallel`, `:pa` | enable or refresh automation and open the focused Parallel Operations board |
| Parallel peek | `:peek` | inspect active parallel agent conversations |

Global keys include `Ctrl+q` to exit, `Ctrl+t` to start a blank draft, and `Esc`/`Ctrl+c` to close
the active inspection surface. A modal may override global keys while it owns focus; visible key
copy must match the current controller path.

## Kernel JSON CLI

`akra describe --json` prints the embedded kernel specification without starting a kernel.
`akra kernel <scope> <command> --field value --json` sends a command through the shared
application command port. `--workspace <directory>` selects another workspace.
Command names, scopes, and input fields come from the specification.

`akra kernel query <name> --json` reads kernel state. `akra kernel history --after <sequence>
--wait --json` waits for records. `akra kernel operation <op_id> --wait --json` waits for an
operation to finish. Output is one JSON line. Exit codes are 0 for acceptance, 2 for rejection,
and 1 for invalid input or an unavailable connection. Acceptance is not completion.

Mutations accept `--idempotency-key`. An omitted key is random and lasts for one invocation.
Callers that retry across invocations must supply the same key. Destructive commands require
`--confirm true`. When a command ends with an error, the output carries `idempotency_key`, and
`error.code` when the kernel answered with a JSON-RPC error. The caller sends the same command
again with that key; the kernel does not run it twice.

The Gateway path uses a transient attachment. A transient last client releases an idle kernel
without the durable client's ten-minute grace. With no Gateway, stored commands run under a
command-owned repository lock without starting app-server, schedulers, or prompt recovery.
Runtime commands are rejected with `kernel_unreachable` in three cases, and `message` names the
case: no Gateway runs, the Gateway runs but another process (a TUI) runs the kernel of that
repository, or the running Gateway is an older version than the CLI (it refuses the attach
request; restart the Gateway). In all three cases a stored command runs under the repository lock. The existing TUI-owned reset compatibility path remains until K6 and does not create
kernel audit records.

A kernel that only this CLI started stops when the command ends. The session list is not kept
between invocations, so `load_sessions` helps only while another client (Akra Desktop) keeps the
kernel up. The automatic follow-up settings are kept: the kernel records them
(`auto_follow_configured`) and the next Gateway kernel restores the last record.

The planning reset of the other entrances also goes to the repository kernel. `akra reset`, the
admin reset (`/api/planning/reset`, `/admin/controls/reset`), and the Telegram `/reset` commands
send `reset_planning` through the same command port and wait for the end of that operation. The
record of the command names the entrance in `caller` (`akra-cli`, `akra-admin`, `akra-telegram`).
The text that each entrance shows did not change: the files that a reset rewrites and removes are
fixed for each target, and a failed reset shows the cause that the kernel recorded
(`command_completed.detail`, at most 300 characters). A folder that cannot have a repository
kernel (a repository without a primary checkout, a parallel slot worktree) still resets directly. The confirmation rule of the entrances did not change either: a queue
reset needs no confirmation, `directions` and `all` need one, and an unconfirmed request is refused
before it reaches the kernel. Under a TUI that runs the kernel, the reset still runs directly and
leaves no record (until K6).

The generated `akra-kernel` skill is installed independently of MCP server registration and
registered through `skills/extraRoots/set`. Its source is `schema/akra-kernel-spec.json`.
`node scripts/generate-kernel-skill.mjs` prints the generated document; `--check` verifies the
shipped asset.

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
:think <none|minimal|low|medium|high|xhigh|max|default>
:copy [selection|last]
:planning [doctor]
:doctor
:reset <queue|directions|all>
:new
:help
```

`:model` opens a staged `Model → Reasoning` picker. It shows the active provider without
inventing unavailable providers, then exposes only the selected model's reasoning choices and
marks its recommended level. The OpenAI catalog begins with GPT-5.6 Sol, Terra, and Luna; its
GPT-5.6 rows offer `max` and omit `minimal`.

The initial interactive defaults are GPT-5.6 Sol with `medium` effort. `:model` and `:think`
change the current selection immediately and persist the next-new-thread default in the global
configuration; an active thread also retains its selected model and effort independently. Reopening
an older or external thread without Akra's saved options deliberately leaves both fields unset for
the app-server rather than applying a newer global default. See the
[Configuration Reference](configuration.md) for the layered file contract and CLI.

`:turns` controls single-session auto-follow. Parallel automation is a separate explicit opt-in.
`:stop` terminates active app-server sessions, closes the active parallel epoch, and disarms both
continuation paths. A later `:parallel` re-arms only parallel continuation.

In the conversation, a left-button drag selects the exact rendered transcript cells, keeps their
background highlighted, and copies the resulting text through OSC 52 when the button is released.
The clipboard path supports direct terminals and tmux passthrough. `:copy selection` repeats the
last completed selection; `:copy last` copies the latest raw assistant answer. Akra owns mouse
reporting for the entire fullscreen lifecycle so wheel scrolling, cards, and transcript selection
behave consistently. There is no runtime or startup capture switch, and terminal-native drag
selection is not a supported Akra fullscreen interaction.

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

The default app-server execution profile is `approval=never`, `reviewer=none`, and
`sandbox=danger-full-access`. Main conversations, resumed threads, hidden planning workers, and
parallel workers use that same profile at both thread and turn boundaries. The project is still
marked untrusted in app-server configuration so repository-local configuration cannot silently
replace Akra's process contract. Diagnostics always show the active profile, but the intentional
default is informational rather than a permanent degraded warning.

Codex discovery still pins an absolute launcher, native interpreter, and canonical package script
before spawn. Ownership, ACL, or repository-boundary trust failures are non-blocking by default and
remain visible as startup warnings; launcher shape, native format, absolute-path, and package-target
validation still apply. A principal with mutation rights can still replace a permissively accepted
path before spawn. Set `AKRA_REQUIRE_TRUSTED_CODEX_EXECUTABLE=1` to restore the strict fail-closed
trust boundary and block that exposure. An absent value or exact `0` keeps the permissive default,
while any other value rejects startup as invalid configuration.

Deployments can opt down with `CODEX_EXEC_LOOP_APP_SERVER_APPROVAL_POLICY`,
`CODEX_EXEC_LOOP_APP_SERVER_APPROVALS_REVIEWER`, and
`CODEX_EXEC_LOOP_APP_SERVER_SANDBOX_MODE`. When approvals are enabled, only the interactive main
conversation can answer a reviewable command or bounded additional permission request. `Y`
accepts once; `N`/`Esc` declines; `Enter` is inert. Timeout, interrupt, disconnect, invalid
payloads, uninspectable file-change requests, and unattended workers fail closed. No session-wide
grant is cached.

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
- Hidden planning workers inherit the active app-server execution profile. Their developer
  contract remains planning-only, and the host validates and applies their structured commands.

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
- Workers inherit the active app-server execution profile; the default is unattended
  `danger-full-access` with no approval prompts. They leave edits uncommitted, and a restrictive
  profile still declines approval requests in unattended connections.
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

Selection is keyed by slot/session identity and survives projection reorder on refresh. With an
empty task composer, `Tab`, `Shift+Tab`, and the arrow keys navigate the board and `Enter` inspects
the selected lane. `Ctrl+R` refreshes, `Ctrl+P` disables parallel mode, `:peek` opens the read-only
active-agent picker, and `Esc` closes the focused board. Once task input is non-empty, navigation,
printable keys, whitespace, and `Enter` belong to the composer so work can be submitted without
leaving Parallel Operations. The composer remains hidden and locked only while the supervisor
projection is loading; an existing draft is preserved until loading finishes. A genuine readiness
blocker remains the board's first actionable fact after loading and does not take task input away.
Closing preserves the draft while the enabled passive projection continues. The 80-column layout
stacks lanes before selected detail, while 120- and 160-column layouts show lifecycle, lanes, and
detail together; each ready focused layout reserves a composer tail.

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

Post-merge PR validation is a durable application scheduler, not an Admin or TUI refresh loop. An
unset repository-local `akra.prValidationMode` defaults to `observe`: provider evidence and exact
Actions target SHAs are recorded, but automatic Queue admission is blocked. `remediate` admits only
typed actionable findings through the ordinary Planning Queue and worker lease/worktree lifecycle;
`off` stops new polling without deleting history. The Admin Validation Rail distinguishes
Integrated, Verifying, RemediationQueued, RemediationRunning, Verified, Blocked, and Failed, and its
rollout banner states the effective mode, Queue admission policy, and separate Ruleset approval
boundary. See [PR Validation Rollout](pr-validation-rollout.md).

Rollout timing evidence follows a separate read path. The application query service validates the
collector schema, repository/base identity, generated time, evidence SHA, and 48-hour freshness
boundary before projecting actual and historical Fast Gate distributions, CI/Post-Merge timing,
sample scope, quota, and typed warnings. The repository authority retains at most 64 redacted
snapshots and exact collection lease/CAS state. Dashboard bootstrap contains only the latest
summary; the evidence drawer lazily requests at most 20 stable, cursor-ordered history rows. SSE
contains only evidence revision invalidation. A durable replay recalculates freshness, and a source
or store outage degrades to the last valid read-only projection without stopping the parallel
control plane.

The `/admin/akra` game projection uses a PixiJS 8 world over the same dashboard snapshot. Its
validated frontend store, semantic camera zoom, worker movement, furniture occlusion, and scene
selection remain presentation-only; see [Admin Game Frontend](admin-game-frontend.md).

For UI/UX debugging, `akra admin --debug-harness` starts an application-owned, non-durable Fake
scenario clock behind the same dashboard contract. It supports normal delivery, blocked recovery,
queue pressure, and the ten PR-validation recovery scenarios without mutating real planning,
parallel, Git, or GitHub authority.

## Gateway Daemon

`akra-gateway` is a resident daemon that relays local clients (the planned Akra Desktop) to a
Gateway-owned `codex app-server`. It is P1 of [the Akra Desktop plan](../plan/17-akra-desktop/README.md);
the desktop app itself is not shipped yet.

- Endpoint: unix `$AKRA_HOME/run/gateway.sock` (directory `0700`, socket `0600`); Windows named pipe
  `\\.\pipe\akra-gateway-<user SID>` with an owner-only security descriptor, first-instance creation, and
  remote clients rejected. No TCP port is opened. Startup prints `akra-gateway listening on <endpoint>`.
- Upstream: the Gateway runs `codex app-server --listen unix://$AKRA_HOME/run/codex-app-server.sock`
  with the same pinned executable, `-c` security overrides, and scrubbed environment as the TUI. It
  restarts an exited upstream with 1 s to 30 s backoff and reports the count in `akra/gateway/info`.
  It records the upstream pid in `$AKRA_HOME/run/gateway-upstream.pid`; on unix a new Gateway stops an
  upstream that a crashed Gateway left behind (only when that process is still an `app-server` for the
  same socket) before it starts its own.
- Sessions: each client WebSocket gets its own `codex app-server proxy --sock …` child, so `initialize`,
  request ids, and thread subscriptions stay per connection. Frames pass through unchanged; only
  `akra/*` requests are answered by the Gateway (`akra/mcp/*` is the MCP hub and `akra/context/*` is the
  work context, see below;
  `akra/gateway/info` returns gateway and codex versions,
  start time, and upstream restarts). An upstream restart closes clients with WebSocket close `1012`;
  an unavailable upstream closes them with `1013`.
- Hosts (plan 25 S1a): a connection selects its upstream host with the WebSocket request path
  `/host/<id>`; any other path means `local`. Production registers `local`, the app-server above, and
  `cloud` only when `AKRA_DOTS_CLOUD` is `1`/`true` or the flag file `$AKRA_HOME/dots/cloud-enabled`
  exists; the Gateway reads this once at startup. Desktop's command "Dots 켜기 / 끄기" writes or
  removes that flag (`desktop/src/main/dotsSettings.ts`) and turns the sidebar's remote section on or
  off with it, so the user enables dots in one step. At startup Desktop also turns the section on when
  the flag exists and the user never chose a value for the section. The cloud host appears on the next Gateway start,
  without dropping local sessions.
  An unknown id is closed with `1008`. `akra/gateway/info` also returns `host` (the host of this
  connection) and `hosts` (id, codex version, start time, and restarts of each host). On a connection
  to a host other than `local`, `akra/kernel/*` is refused with `-32601`, `akra/context/*` answers
  with empty `resources` and does not read or write the ledger, `akra/mcp/*` is unchanged, and
  `AKRA_GATEWAY_RECORD` records nothing.
- Cloud host (D-19): `/host/cloud` bridges to the OpenAI cloud app-server with the access token of
  `~/.codex/auth.json`. The bridge forwards only the public-protocol methods it allows
  (`initialize`, `initialized`, `thread/list`, `thread/read`, `thread/turns/list`,
  `thread/items/list`, `model/list` for reading, and `thread/resume`, `thread/unsubscribe`,
  `turn/start`, `turn/steer`, `turn/interrupt`, `thread/stop` for a conversation), plus error and
  result responses. It answers every other request with `-32601` and drops other notifications and
  binary frames. It never hand-writes a private method such as `turn/addUserMessage`. The bridge
  allows those write methods, but Desktop has no control that sends them: cloud threads are read-only
  in the UI. When the
  cloud rejects the login (HTTP 401 or 403) the connection closes with `1008` and reason
  `cloud login rejected`. The bridge pings the cloud every 10 seconds and ends the session after 60
  seconds of silence (`src/adapter/outbound/app_server/cloud_upstream.rs`).
- Dot list (D-19): while the cloud host is enabled, `akra/dots/list` answers `{ dots }` with the id,
  display name, status, kind, paused flag, and current work thread of each cloud agent. The Gateway
  reads them with `GET https://chatgpt.com/backend-api/tbo` through `curl -q --config -`, so the token
  stays out of argv, disk, and logs; it reads the first page only. When the cloud host is off the
  method is unknown (`-32601`). Desktop asks once per remote connection. The sidebar shows two
  sections for the host, above the local folders: "Dots" holds the dots and, folded under "작업 대화",
  the work threads that an agent made (`threadSource` starts with `aeon`); "클라우드 · 읽기 전용" holds
  the cloud threads that the user made (`src/adapter/outbound/app_server/cloud_dots.rs`,
  `src/adapter/inbound/gateway/dots.rs`).
- Dot chat (D-19): `akra/dots/messages` with `{ roomId }` answers `{ messages }`, the latest 30
  messages of that message room, oldest first, each with id, time, author name, a from-dot flag, and
  text; deleted messages are left out. The room id must be letters, digits, `-`, or `_`. Desktop opens
  this chat when the user clicks a dot. The chat is read-only until the user clicks "보내기 허용" and
  confirms a warning; the permission holds for that dot until the app closes. Then a composer
  appears and `akra/dots/send` with `{ roomId, text }` posts the trimmed text (1 to 8000 characters) to
  the room with a fresh `request_id` and answers `{ sent: true }`. When the service answers HTTP 400 or
  more, the error names the status and the service's short reason. Desktop reads the room on open, on "새로 고침", and when the dot finishes a reply (live state, below).
  Only while the live channel is down, after a send it reads again every 2 seconds until the newest
  message is a new one from the dot, for at most 45 seconds. The answer also carries `dotReadAt`, the
  time the dot last read the room; the chat shows "읽음" under the user's newest message before it.
- Dot live state (D-19): `akra/dots/watch` with `{ roomId }` starts a watch for that connection and
  answers `{ watching: true }`; `akra/dots/unwatch` stops it; a connection watches at most 8 rooms and
  its watches end with it. The Gateway then sends `akra/dots/live` notifications with `roomId`,
  `connected`, `working`, `activityLabel`, and `usingComputer`. It reads them from the service: it
  gets a signed WebSocket address from `GET /celsius/ws/user`, sends `connect` (presence `background`)
  and `subscribe` to topic `calpico-room-live:<room>`, then holds `POST /messaging/rooms/{id}/live`
  (SSE, through curl) open; the service pushes `calpico-room-live-state` on the WebSocket while that
  stream is open. It reconnects after 5 to 60 seconds and reports `connected: false` in between. It reports
  `connected: true` only with a real room state: it sends nothing on its own when the channel comes
  up, and it skips a state whose threads are all still `attaching`, because that state does not know
  the turn status yet.
  Desktop watches the room of every dot. A room that goes from working to idle counts as a reply:
  Desktop reads the chat again, marks the dot "새 답" in the sidebar until the user opens it, and shows
  an OS notification without the message text when the user is not looking at that chat. While the
  dot works, the chat shows a typing mark and the header shows the service's activity label
  (`src/adapter/outbound/app_server/cloud_dots_live.rs`). A real send was verified once on 2026-11-14.
  The header shows "새로 고침" only while the live channel is down
  (`desktop/src/renderer/views/DotChat.tsx`, `desktop/src/renderer/views/dotSend.ts`).
- Dot side chat (D-19): the chat with a dot is not a main view. A click on the dot in the sidebar opens
  it in a resizable column at the right edge of the main window, beside the thread, home, or fleet
  view; a second click or the header's close button closes it. Below a window width of 1400 px the
  side chat and the right panel do not fit together, and the one opened last stays
  (`desktop/src/renderer/views/DotDock.tsx`, `desktop/src/renderer/session/uiStore.ts`).
- Dot window (D-19): the side chat's "따로 뜬 창으로 옮기기" moves the chat to a 440 by 680 window that
  shows only that dot, and closes the side chat. The window's "본 창의 옆 칸으로 옮기기" moves it back
  to the main window that opened it and closes the window. The main process keeps one window per dot;
  a click on that dot in the sidebar focuses the window. The window loads the same renderer with
  `?dotWindow=<host>&dot=<id>`, opens its own remote Gateway connection, does not connect to `local`,
  and uses the main window's light or dark choice (stored in `localStorage`). The main process holds
  the send permission per dot while the app runs, so the side chat and the window ask once. While a
  dot's window is open, that window shows the reply notification and the main window does not
  (`desktop/src/renderer/views/DotWindow.tsx`, `desktop/src/main/index.ts`).
- Desktop remote hosts (plan 25, Desktop part of S2): Desktop reads `hosts` from `akra/gateway/info`. The
  command palette has one command for the remote section, "Dots 켜기 / 끄기" (above). The
  default is off, and turning it on shows a warning first. While it is on, each window opens one more
  Gateway connection per remote host on `/host/<id>` and the sidebar shows a section for that host,
  above the local work folders, with
  its threads. A remote thread opens read-only with `thread/read` and `thread/turns/list`. Desktop has no
  control that sends a turn, a resume, or an approval answer to a cloud thread; the user talks to a dot
  in the dot chat instead. The remote conversation
  does not use the kernel, changes, documents, or terminal panels. Remote state stays in memory. A connection that the Gateway closes with `1008` is not
  retried; the section shows a login message when the reason is `cloud login rejected`. The command and
  the section appear only when the Gateway registers a remote host
  (`desktop/src/main/gatewayConnection.ts`, `desktop/src/renderer/session/`).
- Kernel protocol (`akra/kernel/*`, plan 18 K3, protocol version 1 since the kernel spec of K4-1): a client sends `akra/kernel/attach`
  with a workspace and attaches to the kernel of the repository that holds it. The Gateway builds that
  kernel on first attach and keeps it after clients leave, so auto-follow and parallel dispatch keep
  running. Worktrees of one repository share one kernel, which works in the primary checkout. An
  attach from an Akra parallel slot worktree, or to a repository without a primary checkout (a bare
  repository), fails. If another process (a TUI) runs the kernel of that
  repository, attach fails with `-32012`. After attach the client sends `akra/kernel/command` and
  `akra/kernel/query` and receives `akra/kernel/event` notifications; requests before attach fail
  with `-32011`, and a kernel that does not answer in time fails with `-32013` (a command may still
  run). No event arrives after the `akra/kernel/detach` answer. `akra/kernel/describe` answers with the
  kernel spec (protocol version 1): `methods`, `commands` (scope, summary, effect, idempotency, possible
  rejections, input and output JSON Schema), `queries`, `events`, `records`, `reasons`, `errors`, the meaning
  of the scope, effect, and idempotency values (`terms`), and the shared type definitions in `$defs`
  (JSON Schema 2020-12). The input schemas carry the rules the kernel checks (non-blank text, the
  length of `idempotency_key`). The state values of the parallel board (agent state, merge queue
  state, pull request validation state and phase) are closed lists in the spec. Fields have no description in the spec yet. The same value is checked
  in as `schema/akra-kernel-spec.json`. All reason codes are one list (`reasons`): command
  rejections and the reasons of `prompt_submission` events use the same names. Each reason and each
  error has `next`, an ordered list of steps a client can run as given: `wait_for { event }`,
  `wait { milliseconds }`, `query { name }`, `command { name }`, `method { name }`, `resend` (the same
  request; a command gets a new key), `ask_user`, and `stop`. A `wait_for` step has no condition, so
  the client matches the event to its prompt by `task_id`. A command rejection carries `reason`, `message` (the summary of the
  reason), and the `next` of that reason. A `prompt_submission` event with a reason carries the same
  `next`. When the kernel drops a queued prompt without a turn, the client gets `prompt_submission`
  with status `dropped` and reason `task_finished` (its task is already done or cancelled) or
  `rejected_again` (the one resend was refused again; the task stays queued). The `planning` and
  `parallel` queries and events carry the planning
  queue and the parallel state (pool, agents, merge queue, PR validation); `refresh_planning` reads the
  queue again. The `history` query (`{"type": "history", "after": <cursor>, "limit": <n>}`) pages
  through the kernel decision log in record order and answers with `records` and `next_after`; send
  `next_after` as the next `after`. Each new record also arrives live as a `recorded` event with the
  same `sequence`; after `resync`, read `history` after the last sequence seen. Approval requests and
  their ends are records (`approval_requested`, `approval_resolved` with `accepted`, `declined`,
  `timed_out`, `withdrawn`, or `lost`); a restarted kernel records the approvals of the previous kernel
  as `lost` and cannot answer them. `akra/kernel/command` accepts an optional `idempotency_key`
  (1 to 128 characters) and answers a recorded command with
  `operation { op_id, state, next { query, after }, replayed }`: `op_id` is the sequence of the
  `command_received` record, `state` is `accepted` or `rejected` (the result of the admission), and
  the progress of the command is the log after `op_id`. The `operation` query
  (`{"type": "operation", "op_id": <id>}`) answers with the current `state` of that operation and its
  `records`: `accepted`, `running`, `done`, `rejected`, `failed`, `lost` (the kernel was built again
  while the operation was open), or `unknown` (no command record has that id, or it is older than the
  retention limit). The kernel computes the state from the records. The `command_received` record
  carries `caller`, the name the connection gave in `akra/kernel/attach` (`client_name`, at most 64
  characters; it is not authentication), so the log says which client sent a command and which
  client decided an approval. The kernel, not the inbound adapter, checks the `effect` of the spec: a
  `destructive` command without `confirm: true` fails with `-32010`, is not run, and is not recorded.
  The `command_received` record also
  carries the `completion` rule that applied when the kernel received the command; a command received
  by an older kernel has none and stays `accepted`. A `submit_prompt` that the kernel still prepares
  is `accepted`; it becomes `running` with its first later record. Each command in the spec has
  `completion`: `accepted` (it ends when the kernel accepts it), `records` (it ends with a later
  record: `submit_prompt`, `rename_session`, `steer_turn`, `stop_all`, `decide_approval`, and
  `reset_planning`), or `unreported` (no command of this version; it is the rule of commands that an
  older kernel received, and their state stays `accepted`). `rename_session`, `steer_turn`,
  `stop_all`, `decide_approval`, and `reset_planning` end with `command_completed` when the request
  of the command returns: `done` if it succeeded and `failed` if it did not. A failed request
  writes its cause into `command_completed.detail` as text for a person. `stop_all` ends with its
  first stop request; the kernel may send the stop again after a turn starts. A command that the
  kernel accepts but that does nothing (a rename while the session list loads, a stop while another
  stop is in flight, a reset while a planning operation runs) ends as `failed` at once. A
  `submit_prompt` ends with the `turn_ended` record of its turn, or with
  `command_completed` when no turn ran (failed preparation, hand-over to parallel slots, a turn
  submission that closed before a turn started), or with the settlement of its queued prompt. A
  queued prompt is `running`, not an end. The records of a `submit_prompt` carry its `op_id`
  (`thread_prompt_queued`, `thread_prompt_settled`, `turn_submitted`, `turn_started`, `turn_ended`).
  A restarted kernel records the turns of the previous kernel as `turn_ended` with `lost` and its
  other open operations as `command_completed` with `lost`; a queued prompt keeps its operation and
  the new kernel submits it again. A resend with the same key and the same command does not run again and gets
  the first receipt with `replayed`; the same key with another command fails with `-32014`. A refused
  command keeps its refusal for its key; send a new key to try again. Keys are shared by all clients
  of the repository kernel, so a client uses a random key for each command. Keys are remembered while
  their record is in the log (the latest 10,000 records), also across a kernel restart that wrote its
  records; a crash can lose the latest keys. A kernel without a log refuses a command with a key
  (`-32015`). A `submit_prompt` that the kernel refuses before preparation (`busy`,
  `preparation_active`, `unknown_thread`, `parallel_mode`, `parallel_mode_unknown`) now also answers
  with `rejection` and `state: rejected`. `load_sessions` and `refresh_planning` are not recorded and
  have no receipt. While a client is attached the kernel refreshes the parallel supervisor snapshot, and the
  first attach runs one supervisor inspection that does not change the pool (it runs the push readiness
  check, like the TUI parallel view).
- A repository kernel with a log observes the git state of its workspace: the checked-out commit and
  branch. A separate thread reads it about every 30 seconds; when it differs from the last record, the
  kernel writes a `repository_changed` record (`head`, `branch`, `previous_head`, `previous_branch`).
  One record can cover several changes. The record does not say who made the change, so commits by an
  Akra turn, by the user, and by an agent that Akra did not open look the same. The record has no thread
  (`thread_id` is empty in `history`). A new kernel starts from the last recorded state, so a change made
  while no kernel ran is one record; the first observation in a log has no `previous_head`. The newest
  `repository_changed` record is kept when old records are trimmed. Other
  worktrees of the repository, other branches, and uncommitted file changes are not observed. The TUI
  runtime does not observe.
- A kernel with no attached client and no remaining work (turns, approvals, auto-follow, evaluation,
  planning work, parallel mode on, or running parallel work) for 10 minutes is unloaded, and its
  repository lock is free for a TUI. The next attach starts it again. Gateway shutdown stops the
  kernels before the upstream app-server.
- The kernel uses the Gateway upstream app-server through its own `codex app-server proxy`
  connection, so a client can `thread/resume` a thread that the kernel runs and receive its turn
  notifications live.
- One Gateway runs per user: a second instance exits with `another akra-gateway is already running`.
  Child processes are contained (Windows job objects) and end with the Gateway.
- `AKRA_GATEWAY_RECORD=1` records each session's text frames as JSONL under the user-private
  `$AKRA_HOME/recordings/`. Recordings contain real conversations and must not be committed.
- The TUI, Admin, and Telegram paths keep their own stdio app-server (plan P-08).

## MCP Hub

The MCP hub keeps one user-level set of MCP servers and their logins for every session and for Akra
itself ([plan 21](../plan/21-akra-mcp-hub.md)). Sessions do not load the tool schemas; they find and
call tools on demand.

- `akra mcp servers|add|remove|login|search|describe|call` prints JSON. `add <name> --url <https-url>`
  registers a streamable HTTP server (plain http only for localhost); `add <name> -- <command> [args...]`
  registers a stdio server. `call <server> <tool> [<json-object>|-]` reads the arguments inline or from
  stdin and exits with 1 when the tool result has `isError: true`.
- The command does not resolve repository configuration, so it runs in any directory.
- Engine: a hub-only `codex app-server` with `CODEX_HOME=$AKRA_HOME/mcp-hub`. When `akra-gateway` runs,
  `servers`, `search`, `describe`, and `call` go to its resident engine (`akra/mcp/statuses`,
  `akra/mcp/call`); otherwise each command starts its own engine and stops it when done. Codex owns the MCP transports, OAuth, and token storage and refresh. The
  registry is `$AKRA_HOME/mcp-hub/config.toml` in Codex `[mcp_servers.<name>]` form; extra Codex keys
  written by hand are kept.
- `call` opens an ephemeral thread with only the target server enabled. `search` and `servers`
  connect every enabled server. Requests that app-server sends to the hub (MCP elicitation and others)
  are declined.
- `login` prints the authorization URL on stderr and waits up to 10 minutes for the OAuth callback.
- The Gateway starts the resident engine on the first request, reuses one ephemeral thread per server,
  handles one request at a time, drops the engine after an error, and stops it after 10 idle minutes.
  `akra mcp` sends `hubHome` with every request; a Gateway that serves a different hub answers `-32040`.
  Requests without `hubHome` (Desktop) use the Gateway's own hub. The CLI
  falls back to its own engine only when it cannot connect, gets method-not-found or `-32040`, or the
  Gateway closes with 1013 before reading; any other failure after sending is an error, so a tool call
  never runs twice. `add`, `remove`, and `login` run in the CLI and then send `akra/mcp/reload`.
- When the registry has at least one server, the hub writes
  `$AKRA_HOME/mcp-hub/agent-skills/akra-mcp-hub/SKILL.md`, and every Akra app-server connection
  registers that folder with `skills/extraRoots/set` right after `initialize`. A failure only leaves
  a warning. Connections opened before the first server was added do not see the skill.
- `akra mcp skill install` writes the same skill to the user's Codex home
  (`$CODEX_HOME/skills/akra-mcp-hub/SKILL.md`, default `~/.codex`) and adds the full path of the `akra`
  executable. Codex sessions that Akra did not open read that folder, so they can use the hub too.
  `akra mcp skill status` and `remove` print the same JSON (`installed`, `path`); `remove` deletes only
  that file and the folder when it is empty. While the installed copy exists, Akra connections do not
  register the hub folder, so a session sees the skill once.

## Work Context

The Gateway records the external documents that a thread mentions, without a model call
([plan 22](../plan/22-akra-work-context.md)).

- `akra/context/thread { threadId }` reads the thread items that are new since the last call
  (`thread/items/list`, newest first, stops at the last read item), finds documents in user and agent
  messages, commands, and MCP tool arguments, stores them, and returns
  `{ resources: [{ kind, key, url, title, source, firstSeenAt }] }`. Command output and tool results are
  not read.
- Recognizers (`src/domain/work_context.rs`): Notion pages, Jira issues (`<site>/browse/KEY-123`), and
  GitHub pull requests and issues. A new service is one `ExternalResourceRecognizer`; the ledger, the
  Gateway, and Desktop handle only the kind name.
- Ledger: `$AKRA_HOME/work-context/ledger.sqlite` with documents, document-to-scope links (only the
  `thread` scope today), scan markers, the last checked version of each document, and the version each
  scope read.
- Desktop reads the open thread's documents when a thread opens and when a turn ends, shows the count
  on the "문서" button, and lists them by kind in the "관련 문서" side panel.
- Changed documents: `akra/context/refresh { threadId }` also asks a version provider for the current
  version of each document whose last check is older than 5 minutes (at most 20 documents per call,
  stops after 3 failures) and returns the same list. Each resource has `changed` and
  `versionCheckedAt`. The first version a thread checks is its read version; a later different version
  sets `changed`. `akra/context/markRead { threadId, kind, key }` makes the checked version the read
  version. `akra/context/thread` asks no external service.
- Version providers (`ExternalResourceVersionPort`): only GitHub today. It reads `updated_at` of the
  pull request or issue with `gh api --hostname github.com`, with `AKRA_GITHUB_TOKEN`, `GH_TOKEN`,
  `GITHUB_TOKEN`, or the stored `gh` login. Notion and Jira documents never show `changed`.
- Desktop calls `refresh` after the list is shown, marks changed documents with "바뀜", shows
  `문서 N · 바뀜 M` on the button, and calls `markRead` when the user opens a changed document.
- Limits: ephemeral threads and threads without turns have no items to read; the first read of a very
  long thread covers the newest 5,000 items.

## Recovery and Limits

- Invalid or conflicting planning updates pause continuation and preserve review evidence.
- Store-backed claims allow official refresh and distributor recovery after restart; recovery does
  not hard-reset operator-owned integration history.
- Non-Git workspaces use planning authority but not the full Git worktree pool.
- One Akra TUI runs per repository, including its linked worktrees. The TUI takes the repository
  kernel lock `$AKRA_HOME/projects/<namespace>/runtime/kernel.lock` before it opens the screen; a
  second TUI prints `another Akra kernel already runs for repository …` with the holder pid and exits.
  If the lock file cannot be opened at all, the TUI warns and starts without the lock.
- Planning detail authoring is manual; the `llm-assisted` editor path is disabled.
- Real-terminal evidence is still required for primitive-sensitive fullscreen TUI changes and affected
  restart/distributor/multi-worktree flows.

## Code Entry

- Core: `src/core/`
- TUI: `src/adapter/inbound/tui/`
- CLI: `src/adapter/inbound/cli.rs`
- Admin: `src/adapter/inbound/admin_api/`
- Telegram: `src/adapter/inbound/telegram_bot/`
- Gateway: `src/adapter/inbound/gateway/`, `src/adapter/outbound/app_server/gateway_upstream.rs`
- Work context: `src/domain/work_context.rs`, `src/application/service/work_context.rs`,
  `src/adapter/inbound/gateway/context.rs`, `desktop/src/renderer/work/threadDocuments.ts`
- MCP hub: `src/adapter/inbound/cli/mcp.rs`, `src/application/service/mcp_hub.rs`,
  `src/adapter/outbound/mcp_hub/`
- Desktop (in development, plan P2): `desktop/` — Electron shell that connects each window to `akra-gateway`;
  see `desktop/AGENTS.md`. The "Akra" side panel attaches to the repository kernel of the open thread's
  folder only while it is open, and shows threads with remaining work, auto-follow, parallel work, and the
  planning queue (`desktop/src/renderer/kernel/`). Kernel wire types and command effects are generated from
  `schema/akra-kernel-spec.json` with `npm run generate:kernel-protocol`. The shared kernel command sender
  adds a fresh UUID idempotency key to each mutation and leaves read commands keyless. It does not replay
  requests after disconnect. The panel currently sends only the read command `refresh_planning`;
  conversation and approval actions still use the app-server path.
  The "Notion" view of the "업무" side panel reads the `notion` server of the
  MCP hub through `akra/mcp/*`: recently viewed pages, search, open in the browser (https Notion hosts
  only), and hand-off that fills the new-thread screen's composer with the page (`desktop/src/renderer/work/`).
  The "Projects" view reads one GitHub Project (`owner/number` or its address, kept in the renderer's local
  storage) with `gh api graphql` (`ProjectV2`, `read:project` scope, at most 500 items). It shows the open
  issues, pull requests, and drafts grouped by the Status field and counts the closed ones. The repository
  filter starts at the `origin` of the open thread's folder. Hand-off of an issue fills the composer with the
  issue body, the 10 newest comments, and a `Closes owner/name#N` request, and picks the first workspace folder
  whose `origin` is the issue's repository; without one, sending asks for a folder
  (`desktop/src/main/githubService.ts`, `desktop/src/renderer/work/GitHubProjectPanel.tsx`).
  The "변경" side panel lists the working-tree changes of the open thread's folder and can stage, unstage, discard,
  or open one file; the main process reads the file's state from git again before it acts, asks before a discard,
  and refuses conflicted files, files with a content filter (LFS), and discarding a folder
  (`desktop/src/main/gitStatus.ts`). Expanded tracked text files also show unstaged hunks with individual stage
  and discard actions. Discard asks for confirmation and preserves staged content. Main reads the diff again
  and rejects stale hunk IDs; unsupported files retain the whole-file actions (`desktop/src/main/gitHunks.ts`).
  Below the files it lists the commits that the branch added after its base
  (the PR's base branch, else the branch of `origin/HEAD`), marks the ones that are not pushed, and shows the
  branch's GitHub PR with its CI state through `gh` (`desktop/src/main/gitBranch.ts`). It can push the branch
  to the remote branch of the same name (never a force push) and create a PR with `gh`; the main process asks
  before both and shows the target (`desktop/src/main/gitPublish.ts`)
- Planning: `src/application/service/planning/`, `src/domain/planning/`
- Parallel: `src/application/service/parallel_mode/`, `src/domain/parallel_mode/`
- App-server adapter: `src/adapter/outbound/app_server/`
- SQLite authority: `src/adapter/outbound/db/sqlite_planning_authority_adapter.rs`
