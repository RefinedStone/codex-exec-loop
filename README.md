# Akra

Akra is the native Rust operator client for `codex app-server`.

The repository is still named `codex-exec-loop`, the Rust crate and native binary are still
`codex-exec-loop-native`, and the operator command is `akra`.

Akra is built for long-lived work in one terminal session. It keeps startup diagnostics, session
resume, prompt streaming, accepted planning, queue-driven continuation, and parallel worker
supervision in a single inline shell. The TUI is the primary product surface, but CLI commands,
the local admin API, Telegram control, structured planning tools, and release automation all call
the same application services instead of carrying separate planning logic.

## What Is Implemented Now

| Area | Current implementation |
| --- | --- |
| Native shell | Ratatui/Crossterm inline main-buffer TUI. Completed assistant output is inserted into host terminal scrollback; the live viewport keeps the active prompt, stream tail, overlays, and compact notices together. |
| Core runtime | A headless `src/core` command/effect/completion/snapshot boundary coordinates startup, sessions, turn submission, stream reduction, completions, and post-turn evaluation. |
| App-server integration | `src/adapter/outbound/app_server/` starts and speaks to `codex app-server`; checked-in protocol fixtures live under `schema/`. |
| Planning authority | Git-backed workspaces use a repo-scoped SQLite planning authority under the user-level `.akra/projects/<repo-hash>/runtime/` store. An owner-private incarnation marker in the canonical Git common directory binds repository identity, so linked worktrees and repository moves retain one authority while fresh clones and same-path replacements remain isolated. Planning files under `.codex-exec-loop/planning/` are operator-facing workspace artifacts, drafts, prompts, and mirrors, not the runtime source of task truth. |
| Queue and continuation | Accepted planning decides the executable queue head, proposed work, skip framing, queue-idle behavior, internal `next-task`, and post-turn continuation. |
| Parallel mode | Git-backed supersession mode manages a fixed local `akra` worktree pool, worker leases, session detail, official completion refresh, serialized distributor delivery, GitHub PR automation, frozen reviewed-range integration into a configured branch, and slot cleanup. |
| Operator surfaces | Native TUI, non-TUI CLI, local Axum/Askama admin UI and JSON API, Telegram bot runner, and JSON planning-tool automation. |
| Distribution | Native release archives, npm split packages, platform bundles, validation capture scripts, native PR checks, and tag-triggered GitHub Release/npm workflows. |

## Why It Matters For Open Source Maintainers

Akra is not a general chat wrapper. It is an open-source maintainer workflow client for people who
need Codex to help with repeated repository operations while preserving local control.

- PR review and delivery: parallel mode leases local worktree slots, tracks worker session detail,
  pushes source branches, drives PR automation, freezes the reviewed merge-base-to-tip range,
  integrates every commit in that range into the configured target, and cleans up finished slots.
- Issue and task triage: accepted planning authority turns operator intent, proposed work, and
  runtime task intake into a validated queue rather than a loose prompt backlog.
- Release workflow: native bundle packaging, npm staging, GitHub Release assets, checksum
  verification, and platform validation records are first-class repository surfaces.
- Maintainer safety: GitHub identity verification, loopback-only admin binding, CSRF protection,
  Telegram allowlists, explicit reset confirmations, architecture-boundary tests, and isolated
  worktrees keep automation accountable.
- Codex interoperability: the runtime is built on official `codex app-server` flow while keeping
  adapters, ports, durable planning authority, and terminal UI contracts open for inspection.

Current operator-facing behavior is tracked in
[docs/supersession/current-contract.md](docs/supersession/current-contract.md). Architecture details
are under [docs/design/](docs/design/).

## Install

### Prerequisites

- `codex` CLI is installed on a trusted absolute `PATH`. Akra pins the resolved executable before
  app-server startup and rejects relative, repository/pool-controlled, replaceable, or unsafe-owner
  path components instead of searching past the first hostile match. A native Codex binary is
  accepted directly. On Unix, the supported npm launcher is a bounded Codex script with either
  `#!/usr/bin/env node` or an absolute `node`/`nodejs` shebang; Akra resolves and pins the native
  Node interpreter separately. On Windows, only a bounded standard npm `.cmd` shim that resolves to
  `node_modules/@openai/codex/bin/codex.js` is accepted, again with a separately pinned native
  `node.exe`. Arbitrary shell, batch, or interpreter launchers fail closed.
- `codex login` has already completed for the operator account.
- The workspace you run `akra` from is the workspace you want Akra to operate on.
- Git-backed planning and parallel mode expect a normal Git repository.
- Parallel GitHub delivery requires `AKRA_GITHUB_LOGIN=<login>` or repo-local
  `git config akra.githubLogin <login>`, plus a credential-free GitHub HTTPS push remote. Frozen
  delivery snapshots a parent token (`AKRA_GITHUB_TOKEN`, `GH_TOKEN`, `GITHUB_TOKEN`) or `gh auth
  token`, verifies that token against the pinned login, and uses it only in a private child-process
  credential helper. Repository/global credential helpers and transport rewrites are not imported
  into autonomous delivery, and token-bearing URLs are never persisted or placed in argv.

### npm

```bash
npm install -g @refinedstone/akra
cd /path/to/workspace
akra
```

The npm package uses a small JavaScript launcher plus platform-specific native optional
dependencies. Supported npm targets are:

- Linux `x64`
- macOS Apple Silicon `arm64`
- Windows `x64`

Node.js `>=18` is required for the npm launcher. The TUI itself runs in the native Rust binary.

### Source

```bash
git clone https://github.com/RefinedStone/codex-exec-loop.git
cd codex-exec-loop
. "$HOME/.cargo/env"
cargo install --path . --bin akra --locked

cd /path/to/workspace
akra
```

### Local Build

```bash
git clone https://github.com/RefinedStone/codex-exec-loop.git
cd codex-exec-loop
. "$HOME/.cargo/env"
cargo build --release

cd /path/to/workspace
/path/to/codex-exec-loop/target/release/codex-exec-loop-native
```

### Native Release Bundle

```bash
cd /path/to/codex-exec-loop
./scripts/package_native_release.sh
```

The bundle contains the native binary, an `akra` launcher or `akra.cmd`, runtime app-server skill
assets, `scripts/gh-akra.sh`, `README.md`, `OPERATOR.md`, guarded
[prompt examples](examples/README.md), the embedded admin font's third-party notice, `VERSION.txt`,
and checksum files. See
[docs/plan/13-native-packaging-and-operator-runbook.md](docs/plan/13-native-packaging-and-operator-runbook.md).

## Quick Start

```bash
cd /path/to/workspace
akra
```

1. Let startup diagnostics finish. The shell can render immediately, but prompt submission is gated
   until diagnostics allow it.
2. Type a prompt and press `Enter`, or use `Ctrl+o` or `:sessions` to resume a prior Codex thread.
3. Use `Ctrl+d` or `:diag` when startup, sessions, app-server, or planning readiness is blocked.
4. Use `:planning` to create or reopen the planning control center.
5. Use `:queue` to inspect accepted queue work, proposals, and skipped work.
6. Use `:parallel` or `:pa` in a git-backed workspace when accepted queue work should run through
   the local parallel worker pool.

The normal loop is one shell: draft, submit, watch the live stream, inspect diagnostics or planning,
resume sessions, let post-turn continuation decide whether queue work advances, and optionally
supervise parallel workers without leaving the terminal.

Auto-follow is disabled for every new or resumed conversation. A manual prompt never enables it.
Use `:turns <positive-number>` or `:turns infinite` to opt in explicitly. `:turns off` and
`:turns 0` disable it again. Parallel mode is a separate explicit automation permission and does not
require `:turns`. `:stop` closes active app-server and parallel automation, and subsequent manual
prompts keep both continuation paths disarmed. A later `:parallel` re-arms only parallel dispatch;
single-session auto-follow stays stopped until a positive or infinite `:turns` command re-enables it.

## Native TUI

### Global Keys

| Key | Purpose |
| --- | --- |
| `Enter` | Submit the active prompt, execute a typed `:` command, or confirm the focused action. |
| `Ctrl+j` | Insert a newline in the active prompt. |
| `Ctrl+t` | Start a blank draft. |
| `Ctrl+o` | Open recent sessions, or close/open the supersession board while parallel mode owns the surface. |
| `Ctrl+d` | Open diagnostics. |
| `Ctrl+r` | Rerun startup diagnostics or refresh parallel readiness in the board. |
| `Ctrl+q` | Quit Akra. |
| `Ctrl+c` | Back or cancel inside the app. It is not the primary quit action. |

### Shell Commands

Typed shell commands begin with `:`. A bare `:` opens the command palette; partial command names
filter suggestions.

| Command | Purpose |
| --- | --- |
| `:diag`, `:diagnostics` | Open startup diagnostics. |
| `:sessions`, `:session` | Browse and resume recent sessions. |
| `:queue`, `:q` | Inspect accepted queue work, proposals, and skip framing. |
| `:planning`, `:planning-init` | Open the planning control center. |
| `:planning doctor`, `:doctor` | Inspect planning health without authoring. |
| `:directions` | Maintain direction-side planning artifacts and queue-idle prompt support. |
| `:reset queue` | Reset queue-side planning state immediately. |
| `:reset directions` | Show reset guidance for direction-side planning state. |
| `:reset directions confirm` | Confirm direction-side reset. |
| `:reset all` | Show reset guidance for the full planning scaffold. |
| `:reset all confirm` | Confirm full planning reset. |
| `:turns <positive-number|infinite>` | Explicitly enable auto-follow with a bounded or infinite turn budget. |
| `:turns off`, `:turns 0` | Disable auto-follow. |
| `:model` | Open model and reasoning-effort selection. |
| `:model default` | Reset model selection to app-server defaults. |
| `:think <none|minimal|low|medium|high|xhigh|default>` | Set reasoning effort directly. |
| `:view [simple|medium|detail]` | Choose transcript visibility for tool and status rows. |
| `:language [english|korean]`, `:lang [english|korean]` | Choose TUI language. |
| `:stop` | Stop active app-server sessions, close parallel automation, and disarm both continuation paths. |
| `:new` | Start a new draft. |
| `:parallel`, `:pa` | Enable or refresh parallel mode and open the supervisor board. |
| `:parallel off`, `:pa off` | Disable local parallel mode and close the automation epoch. Worktrees remain in place. |
| `:peek` | Inspect active parallel agents. |
| `:help` | Show shell command help. |

### Shell Modes

| Mode | Entry | What it owns |
| --- | --- | --- |
| Conversation | default | Prompt editing, live stream tail, notices, completed transcript insertion into host scrollback. |
| Diagnostics | `Ctrl+d`, `:diag` | Startup readiness, blocking failures, and next operator action. |
| Sessions | `Ctrl+o`, `:sessions` | Session list and resume selection using the current workspace context. |
| Queue | `:queue` | Current accepted queue head, proposals, skipped work, and continuation framing. |
| Planning | `:planning` | Staged planning authoring, validation, and promotion flow. |
| Directions | `:directions` | Direction docs and queue-idle prompt support. |
| Supersession board | `:parallel`, `:pa` | Parallel readiness, slot pool, roster, selected session detail, distributor head, queue state, and withheld-dispatch reason. |

The interactive TUI handles app-server command-execution and validated additional-permission
approvals in a dedicated modal. Only an explicit `Y` accepts one request; `N`/`Esc` declines it and
`Enter` is deliberately inert. The receipt deadline starts when Akra receives the request; timeout,
interrupt, disconnect, a full approval channel, or an invalid payload declines it, and acceptance
rechecks timeout and interrupt state immediately before the response is committed. No decision is
cached for the session. The modal shows bounded, control-character-normalized
command/reason/path details but never the raw JSON request. Command-level additional permissions and
network targets use the same bounded validation and are shown before approval. Persistent
exec/network policy proposals are validated but never applied by Akra's one-shot `accept` response.
`item/fileChange/requestApproval` is always declined because the current protocol does not provide
Akra a complete reviewable change and grant scope; ordinary edits already permitted by
`workspace-write` do not enter this extra-grant path. Hidden planning and parallel workers have no
interactive operator surface and therefore decline every approval request instead of waiting or
approving unattended.

## Planning Model

Planning is an authority-backed runtime feature, not a side document editor.

- Accepted planning follows `draft -> validate -> promote`.
- In git-backed workspaces, durable task authority lives in SQLite task tables behind
  `PlanningTaskRepositoryPort`.
- Files under `.codex-exec-loop/planning/` remain operator-authored prompts, direction detail docs,
  staged drafts, rejected runtime writes, and review/export artifacts.
- Builtin `next-task` and internal post-turn continuation execute only the accepted queue head.
- Proposed tasks are visible but not executable until promoted or otherwise moved into accepted
  queue state.
- Queue-idle behavior follows accepted direction authority.
- Admin/API task intake creates one validated accepted task and never interrupts an existing
  `in_progress` task.
- Hidden planning workers may refresh queue state through the planning worker boundary, but they do
  not write SQL or tracked planning files directly.

The technical deep dive is
[docs/design/06-planning-runtime-and-draft-editor.md](docs/design/06-planning-runtime-and-draft-editor.md).

## Parallel Mode

Parallel mode, also called supersession in the docs, is the shipped automation path for git-backed
workspaces.

The operator enters it with `:parallel` or `:pa`. The first off-to-on entry checks readiness, opens
the board, resets only proven-clean reusable `akra` pool slots to the remote integration baseline, opens an
automation epoch, and dispatches already-ready accepted queue work up to idle slot capacity.

Important current rules:

- `:parallel on` is not implemented. Use bare `:parallel` or `:pa`.
- Parallel automation is an independent explicit opt-in and does not require single-session
  `:turns`. After `:stop`, a new `:parallel` entry re-arms only the parallel continuation path.
- Re-running `:parallel` while already enabled refreshes readiness and board projection; it does not
  reset the pool or launch a new epoch by itself.
- `Esc`, `Ctrl+c`, and `Ctrl+o` close the board surface without disabling parallel mode.
- `:parallel off` or `:pa off` disables local parallel mode and clears epoch-local dispatch state,
  invalidates any in-flight post-turn dispatch result, and leaves worktrees in place.
- Queue work leases one of three local `akra` slots.
- Parallel workers run unattended with the default `workspace-write` policy and leave source
  changes uncommitted; they do not need write access to linked-worktree Git metadata. After a clean
  `TurnCompleted`, Akra revalidates the exact Running lease, branch, frozen base, and current `HEAD`,
  then creates the local commit with a bounded host-owned Git operation before reserving official
  refresh order.
- Host commit preparation disables inherited Git control environment, fsmonitor, hooks, signing,
  and replacement objects. It does not clear operator-owned index flags: any assume-unchanged,
  skip-worktree, or unmerged entry fails closed. Top-level ignored build output stays outside the
  frozen source commit and is removed only after successful integration at the guarded slot-cleanup
  boundary. Dirty, untracked, ignored, or nested state inside a checked-out tracked submodule still
  fails before the source ref moves, even when repository or submodule configuration asks Git to
  hide that state. Host staging also rejects new or changed gitlinks, hard-linked regular files,
  reparse/special files, symlinked ancestors, files over 64 MiB, and aggregate changed content over
  256 MiB before `git add`; the same file identities are rechecked afterward. Changed paths with
  active clean/process filters, empty or base-equivalent results, merge history, lease/branch/base
  drift, ref races, and a dirty post-commit worktree likewise fail before commit-ready. A clean
  existing linear commit descending from the frozen base remains compatible.
- Before host-owned worktree creation, checkout, reset, cherry-pick, or commit preparation, Akra
  audits the effective repository-local and worktree Git configuration by key name only. External
  execution surfaces such as filter clean/smudge/process, merge drivers, diff commands/textconv,
  interactive diff filters, alternate-ref commands, worktree redirection, archive commands, and
  merge/diff tool commands block the mutation; their values are neither evaluated nor printed.
  Global/system Git config and executable Git environment are already removed at the pinned Git
  subprocess boundary.
- Worker completion becomes distributor-eligible only after hidden official planning refresh marks
  it commit-ready.
- Distributor delivery is serial: source branch push, PR automation, integration into the frozen
  configured target branch, and slot cleanup.
- Before a worker starts, its slot lease freezes the push remote, credential-redacted canonical
  HTTPS URL, GitHub `owner/repository`, repository visibility, integration branch, and fetched
  remote base OID.
  Enqueue carries that immutable target into the queue record with the source branch and source SHA.
  Any later drift or legacy lease/record without the URL proof blocks before a remote write. Git
  fetch/push/ls-remote and PR automation run from an owner-private temporary Git context that reads
  only the frozen URL and the source object directory; repo/global `insteadOf`, proxy, helper,
  `core.sshCommand`, and GitHub routing environment cannot retarget the operation.
- An existing pool is not reset from a newly created or newly advanced remote-tracking ref. The
  fetched integration OID must match the prior local tracking observation; an absent or changed
  observation blocks without slot mutation and requires a second operator-reviewed run. The lease
  then carries the exact fetched base OID through commit preparation, enqueue, review, and detached
  integration rather than trusting the mutable branch name again.
- PR delivery defaults to `required`. The PR must be open, non-draft, target the frozen base/head,
  point `headRefOid` at the frozen source SHA, have review decision `APPROVED`, include at least one
  currently approved review submitted against that exact frozen SHA, have merge state `CLEAN`, and
  have an explicitly passing status-check rollup.
- Review/check gates remain safely recheckable with a bounded polling interval. Transient remote
  failures use durable exponential backoff and stop automatic retries after eight admitted attempts.
- `AKRA_GITHUB_PR_MODE=auto|disabled` (or repo-local `akra.githubPrMode`) can relax PR surface
  selection only when the parent process started Akra with exact
  `AKRA_PARALLEL_AUTONOMOUS_DELIVERY=1`. Repository-local configuration cannot enable autonomous
  delivery, and invalid explicit environment values block.
- Public repository delivery is blocked unless the parent process started Akra with exact
  `AKRA_PARALLEL_ALLOW_PUBLIC_REPOSITORY=1`; repository-local configuration cannot enable it and
  unknown visibility also blocks.
- `AKRA_PARALLEL_INTEGRATION_BRANCH=<branch>` or repo-local
  `git config akra.parallelIntegrationBranch <branch>` overrides the distributor/pool integration
  branch when a repository needs a lane other than `prerelease`; the environment wins.
- The configured integration branch must already exist on the configured push remote. Akra never
  creates or seeds it from the current `HEAD`. Integration runs in a generated detached worktree;
  the operator's canonical checkout and local integration branch are not reset or moved.
- `AKRA_GITHUB_PUSH_REMOTE=<remote>` or repo-local `git config akra.githubPushRemote <remote>`
  overrides the remote Akra uses for branch publish, integration baseline fetch, GitHub repository
  discovery, interactive current-branch review polling, and integration-branch push when `origin`
  is not the correct delivery remote. Invalid or missing explicit remotes fail closed instead of
  falling back to `origin`.
- Recovery is store-backed. Retryable distributor push recovery is limited to source branch push
  failures; integration branch push blocks and local/remote integration divergence remain
  operator-owned, and Akra does not hard-reset local integration history during recovery.
- After integration verification and PR close, Akra deletes the remote source branch only with an
  exact `--force-with-lease=<ref>:<frozen-sha>` guard. A moved branch is preserved and reported.
- Pool reset and cleanup preserve dirty, untracked, pending-operation, unleased non-baseline, and
  non-integrated slot state under both normal and force-disposable policies.
- Every pool mutation acquires a persistent repository-scoped OS lock beneath the private pinned
  pool root and revalidates the root/lock identity before changing a slot. Each newly minted slot
  lease has a CSPRNG-backed generation encoded as exactly 64 lowercase hexadecimal characters.
  Lease, session, lifecycle, distributor, and cleanup events compare that exact generation, so a
  delayed event from an earlier allocation cannot mutate a slot after it has been re-leased. The
  persistent lock file alone does not make a first-use pool look pre-existing. On Unix, a missing
  non-authoritative runtime mirror is recreated only after the authoritative generation CAS; a
  stale mirror must match that same generation, and rollback restores the exact observed body.
- Post-turn work captures both a continuation generation and the active parallel epoch. Long-running
  worker, network, and review operations run outside their commit locks, while each bounded durable
  authority write rechecks and serializes both permits. Closing or replacing either permit before
  that boundary prevents a late result from enqueueing or advancing automation.
- Official refresh claims are serialized by order and exact owner token. Every new owner token
  requires a stable OS process-start identity with the PID; an unavailable or ambiguous probe
  fails closed instead of weakening ownership to PID liveness. Nullable PID-only tokens remain
  read-compatible only for persisted legacy claims. Failure, cancellation, or supersession removes
  only that owner's claim without advancing the order; only a successfully applied host result
  consumes it.

Parallel control-plane architecture is documented in
[docs/design/05-parallel-control-plane-architecture.md](docs/design/05-parallel-control-plane-architecture.md).
The board shape is documented in
[docs/design/08-parallel-mode-supersession-board.md](docs/design/08-parallel-mode-supersession-board.md).

## CLI Commands

Running `akra` with no subcommand starts the TUI. The non-TUI commands are operational entrypoints
over the same planning and parallel services.

| Command | Purpose |
| --- | --- |
| `akra doctor [workspace_dir]` | Read-only planning inspection. |
| `akra status [workspace_dir]` | Print the shared planning status reply. |
| `akra queue [workspace_dir]` | Print the shared queue reply. |
| `akra reset <queue|directions|all> [workspace_dir]` | Rewrite the selected accepted planning scope. |
| `akra planning-tool contract` | Print the compact JSON contract for structured planning-tool callers. |
| `akra planning-tool run [workspace_dir]` | Execute a structured planning-tool request from stdin. |
| `akra parallel-tick [workspace_dir]` | Manually drive the parallel-mode distributor queue. |
| `akra admin [--port <port>]` | Run the local planning/admin web UI and JSON API. |
| `akra telegram [--allow-chat-id <chat_id>]... [--allow-user-id <user_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending] [--rebind-workspace]` | Run the Telegram control-plane runner. The token must come from the environment or private config. |

Compatibility aliases remain where implemented:

- `akra admin-server [--port <port>]`
- `akra telegram-bot [--allow-chat-id <chat_id>]... [--allow-user-id <user_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending] [--rebind-workspace]`
- `akra planning-task-tool <contract|run> [workspace_dir]`

## Admin UI And API

`akra admin` starts a loopback-only Axum server for the current workspace.

```bash
cd /path/to/workspace
akra admin --port 18442
```

The default port is `18442`. The server binds to `127.0.0.1` and requires capability-backed
authentication before every protected page and API request; the data-free login form is the sole
authentication entrypoint. Supply a 256-bit hexadecimal token through
`AKRA_ADMIN_TOKEN`, or let an interactive Akra process generate one from the operating system
CSPRNG and print it once at startup. Non-interactive launches must provide the variable so a token
cannot be written into service or CI logs. The token is never added to a URL.

```bash
AKRA_ADMIN_TOKEN="$(openssl rand -hex 32)" akra admin --port 18442
```

Open the exact login URL printed at startup and submit the token to receive an `HttpOnly`,
`SameSite=Strict` browser session cookie. Each server uses a fresh 256-bit `*.localhost` hostname
and requires that exact hostname and port in `Host`; any supplied `Origin` or `Referer` must resolve
to the same exact origin, and form/API mutations retain their CSRF token checks. `127.0.0.1`, plain
`localhost`, other ports, and alternate loopback names are rejected at the HTTP boundary. This
prevents the host-wide cookie from being delivered to an unrelated loopback service on another port.
Browser sessions rotate on login, expire after 30 minutes idle or eight hours absolute, and can be
revoked with `POST /admin/logout`. Non-browser clients can connect the printed hostname to
`127.0.0.1` and use either
`Authorization: Bearer <token>` or
`x-akra-admin-token: <token>`.

Implemented admin routes include:

- HTML pages under `/admin`, `/admin/directions`, `/admin/tasks`, `/admin/controls`,
  `/admin/drafts`, `/admin/app-server-prompts`, and `/admin/akra/*`.
- JSON planning endpoints under `/api/planning/*`.
- JSON Akra dashboard endpoints under `/api/admin/akra/*`.
- Packaged graphic/game assets under `/admin/assets/*`.

HTML forms use a cookie-backed CSRF token. JSON mutations use the same cookie token mirrored through
the `x-csrf-token` header. The app-server prompt log page remains empty unless prompt logging is
explicitly enabled.

Useful admin environment variables:

- `AKRA_ADMIN_TOKEN=<64 hexadecimal characters>` pins the admin capability token instead of
  generating an ephemeral token at startup.
- `CODEX_EXEC_LOOP_APP_SERVER_APPROVAL_POLICY` and
  `CODEX_EXEC_LOOP_APP_SERVER_SANDBOX_MODE` override the app-server execution policy. The default
  Akra session policy is now `on-request` approvals with `workspace-write` sandboxing.
- `CODEX_EXEC_LOOP_APP_SERVER_APPROVALS_REVIEWER=auto-review` explicitly opts into Codex's automatic
  reviewer; `guardian-subagent` remains a legacy compatibility value. Either may approve tool
  requests without operator confirmation. The default is `user`; requests that reach the
  interactive TUI require an explicit one-shot operator decision.
- `AKRA_APP_SERVER_SHELL_ENVIRONMENT_INHERIT=none|core|all` controls which non-secret parent
  environment variables Codex projects into model-generated shells. The default is `core`;
  invalid values warn and fall back to `core`. Codex's default `KEY`/`SECRET`/`TOKEN` exclusions
  remain enabled for every choice.
- `AKRA_APP_SERVER_PROCESS_ENVIRONMENT=all` explicitly allows the app-server process to inherit
  Akra's complete environment. Without this opt-in, Akra clears the child environment and restores
  only allowlisted OS runtime/terminal variables, CA and transport proxy settings, the
  `HOME`/`CODEX_HOME` paths needed to reuse an existing `codex login`, and non-secret OpenAI endpoint,
  organization, and project routing metadata. Base URLs require HTTPS, except that exact localhost
  and loopback IP HTTP endpoints remain available for local providers. Credentialed or ambiguous
  proxy URLs, and credentialed, ambiguous, or remote plaintext base URLs, are dropped;
  `OPENAI_API_KEY` and `CODEX_API_KEY` are not copied by default. Run `codex login` before Akra when
  practical. Exact `AKRA_APP_SERVER_API_KEY_AUTH=1` is the narrower API-key authentication opt-in:
  it forwards only those two API-key variables while keeping the rest of the scrubbed child policy;
  any other value warns and remains disabled. The explicit `all` override emits a high-risk warning because it exposes
  every parent credential to app-server and its same-user process boundary. Model-generated commands
  always use non-login shells so profile scripts cannot reintroduce filtered variables.
- `AKRA_APP_SERVER_PROMPT_LOG=1` enables durable app-server prompt I/O storage for the current
  process. The default is off so prompt and response bodies are not retained on disk unless an
  operator explicitly opts in. Every production composition started with capture disabled invokes
  an all-row and metadata clear through a private SQLite connection with `secure_delete=ON`; it does
  not intentionally preserve still-unexpired bodies from an earlier opted-in run. Cleanup failure
  emits a body-free warning and does not silently enable capture.
  Only enabled capture retains data: at most 100 interactions, with entries older than seven days
  removed on maintenance, and capture bounded to 16 items per direction and 16,384 Unicode
  characters per body before writing it to the private workspace/repository authority SQLite store.
  Prompt-log bodies are bounded, not redacted, so this opt-in is appropriate only when that complete
  diagnostic content may be retained. Saturated diagnostic capture queues drop excess log records
  without delaying the authoritative UI stream.
- `AKRA_ADMIN_GRAPHIC_ENABLED=0` disables the graphical admin dashboard layer.
- `AKRA_ADMIN_GRAPHIC_POLL_MS=<milliseconds>` sets graphic polling. Values below 5000 are ignored.
## Telegram Control Plane

`akra telegram` runs a local long-polling Telegram bot for the current workspace.

```bash
cd /path/to/workspace
AKRA_TELEGRAM_BOT_TOKEN=<token> \
AKRA_TELEGRAM_ALLOWED_CHAT_IDS=123,-456 \
AKRA_TELEGRAM_ALLOWED_USER_IDS=123,789 \
akra telegram
```

Configuration sources are merged in this order:

1. `$XDG_CONFIG_HOME/akra/telegram.env`, `~/.config/akra/telegram.env`, or native Windows `%LOCALAPPDATA%\akra\telegram.env`
2. Process environment
3. Non-secret CLI flags

The config file must not grant group or other permissions; use `chmod 600` on Unix. On Windows it
must be a single-link file owned by the current user with a protected DACL that grants access only to
that user. Windows parent junctions/reparse points are rejected, and final symlinks are rejected on
both platforms; handle identity and content version are revalidated after the read. `--token` is
rejected because command-line arguments are visible to other local processes.

Supported config keys:

```text
AKRA_TELEGRAM_BOT_TOKEN=<token>
AKRA_TELEGRAM_ALLOWED_CHAT_IDS=123,-456
AKRA_TELEGRAM_ALLOWED_USER_IDS=123,789
```

Supported Telegram commands:

| Command | Purpose |
| --- | --- |
| `/help`, `/start`, `help` | Show help. |
| `/whoami` | Print the current chat/user ids and both authorization states. |
| `/status`, `status` | Planning status. |
| `/queue`, `queue` | Planning queue summary. |
| `/plan [status]` | Planning status namespace. |
| `/parallel`, `/parallel status`, `parallel`, `parallel status`, `/parallel_status` | Read-only parallel dashboard status. |
| `/reset queue`, `/reset directions`, `/reset all` | Reset selected planning scope. |
| `/reset_queue`, `/reset_directions`, `/reset_all` | Reset aliases. |

Planning, queue, reset, review, and parallel status commands require an allowed chat id. Positive
private-chat ids retain chat-only authorization. Negative group/supergroup ids require both the chat
id and the message sender's positive user id in their respective allowlists; service messages without
a sender are denied. Channel posts are not accepted because Telegram does not provide a stable
individual operator identity for that control surface. `/help` and `/whoami` remain available for setup. By default the runner uses
Telegram's negative-offset contract to discard all stale updates before accepting live commands;
startup fails closed if that cursor cannot be established. Pass `--keep-pending` only when replaying
pending commands is intentional.

The durable update cursor and inbox are keyed by Telegram's numeric bot id, so rotating the bot token
does not create a new replay namespace. The runner authenticates that id with `getMe`, then acquires a
private machine-local SQLite stream keyed by the numeric id. The stream permits one live owner for the
OS user, survives workspace rebinding, and retains an explicit canonical workspace binding after a
clean shutdown. Starting the same bot from a different workspace fails closed. After stopping the old
runner, pass `--rebind-workspace` from the intended canonical workspace to move an inactive binding;
an active or concurrently reclaimed binding cannot be moved. A token rotation with the same numeric
id does not require rebinding.

Rebinding does not weaken startup cursor policy. The default still discards every update pending at
startup before accepting live commands. Combining `--rebind-workspace` with `--keep-pending`
explicitly requests that currently pending Telegram commands execute against the newly bound
workspace, so use that combination only for a deliberate handoff. The global gate is renewed and
owner-CAS fenced after every blocking poll and immediately before workspace-local update claims; a
runner that was suspended past expiry discards the returned batch without executing it.

The global stream store is deliberately independent of `AKRA_HOME`: it lives below the operating
system account's stable `~/.akra` root on Unix or LocalAppData on Windows. Directories and the database are
created with current-user-only permissions on Unix and protected current-user-only ACLs on Windows.
The recorded PID, OS process-start identity, owner token, lease generation, and deadline prevent a
reused PID from impersonating the active owner. Linux uses boot ID plus `/proc/<pid>/stat` start time, macOS uses
the BSD process start timestamp, and Windows uses process creation time. A new runner fails closed
when its start identity is unavailable or an existing non-null identity cannot be probed
unambiguously. Nullable PID-only rows remain deadline-fenced only for legacy read compatibility;
owner-token and generation CAS fencing still applies.
Each update is reserved before command or reply side effects. After the owning lease expires, an
interrupted reservation is acknowledged rather than replayed because those external side effects
cannot be committed atomically with the ledger. This is an intentional at-most-once boundary: a
process failure between reservation and the side effect can drop that update, while avoiding a
duplicate destructive command after an ambiguous failure.

## Runtime Files

Akra reads Codex history and sessions from the normal Codex locations, including
`~/.codex/history.jsonl` and `~/.codex/sessions/`.

For git-backed workspaces, accepted planning authority, runtime projections, leases, distributor
state, and session detail are repo-scoped under the user-level `.akra/projects/<repo-hash>/runtime/`
directory. Repository binding installs an owner-private 256-bit incarnation marker in the canonical
Git common directory: linked worktrees for one repository share task, draft, prompt-log, Telegram,
and runtime authority, and moving that common directory preserves the same namespace. Independently
initialized repositories and fresh clones receive a new incarnation even when they reuse the exact
same checkout path, so they cannot silently adopt the deleted repository's local state. Marker
creation is atomic/no-clobber; malformed, linked, permissive, or wrong-owner markers fail closed.
Non-Git workspaces also receive a private workspace-scoped SQLite authority store.

On Unix, the parallel pool's `.leases`, `.distributor-queue`, and `.agent-sessions` JSON files are
non-authoritative recovery/debug mirrors. Their I/O is descriptor-anchored, link-rejecting, bounded,
and atomically installed beneath the pinned pool root. On Windows these pool JSON mirrors are
intentionally absent until an NT relative-handle implementation is available; reads behave as cache
misses and writes/removals are no-op projections while the private SQLite runtime authority remains
fully active. Akra never falls back to following ordinary Windows paths for these mirrors.

Pool allocation, reconciliation, checkout recovery, distributor integration preparation, and
cleanup share the persistent repository-scoped mutation lock. SQLite is the runtime authority;
Unix mirrors are updated only as guarded projections. New lease generations are CSPRNG-backed exact
64-character lowercase hexadecimal tokens, and all authoritative event/cleanup writes use exact
generation compare-and-swap. Legacy generation-less records may be read for bounded compatibility,
but a new lease never mints that weaker form.

Workspace planning artifacts under `.codex-exec-loop/planning/` are still important, but their role
is operator authoring, staged drafts, prompts, rejected-write inspection, review, and export. Do not
treat tracked planning JSON/files as the authoritative runtime task queue for git-backed workspaces.

On Unix, direct planning artifact I/O is descriptor-anchored and removals are atomically preserved
under the ignored `.codex-exec-loop/runtime/planning-quarantine/` retention area. Git-backed file
sync keeps its export revision in the repo-scoped SQLite store, so operational metadata is never
added to the candidate worktree.

On Windows, accepted planning documents, staged drafts, supporting documents already stored in the
repo authority, and rejected archives remain available through repo-scoped SQLite. Direct/non-git
planning filesystem access, candidate inspection, and external file export/apply fail closed; use
the admin draft editor for those workflows.

## Architecture Map

```text
adapter/inbound/* -> core or application -> domain
core -> application -> domain
application -> outbound ports -> adapter/outbound/*
composition -> concrete wiring
```

Key paths:

| Path | Role |
| --- | --- |
| `src/core/` | Headless app runtime: commands, effects, completions, stream reduction, snapshots. |
| `src/domain/` | Pure models, validation, and invariants. |
| `src/domain/planning/` | Planning workspace, directions, tasks, queue, validation, and projections. |
| `src/domain/parallel_mode/` | Supervisor, pool, distributor, runtime event, readiness, and slot/session rules. |
| `src/application/service/` | Use-case orchestration for startup, sessions, conversations, prompt assembly, planning, post-turn evaluation, GitHub review polling, and parallel mode. |
| `src/application/service/planning/` | Planning facade, admin workflows, authoring, composition, control, repair, runtime intake, shared reports, task mutation, task tool, and planning worker orchestration. |
| `src/application/service/parallel_mode/` | Control-plane, supervisor, pool, distributor, session detail, turn orchestration, Git/GitHub delivery, and cleanup. |
| `src/application/port/outbound/` | Application-owned ports for app-server, startup probes, session catalog, planning authority/workspace/tasks, planning workers, interactive runtime, GitHub, git/worktree runtime, parallel workers, event logs, and Telegram. |
| `src/adapter/inbound/tui/` | Native shell, controllers, overlays, rendering, language/theme controls, terminal adapter, and TUI tests. |
| `src/adapter/inbound/cli.rs` | Non-TUI command dispatch. |
| `src/adapter/inbound/admin_api/` | Admin HTML and JSON API. |
| `src/adapter/inbound/telegram_bot/` | Telegram runner, config, message parser, and control-plane mapping. |
| `src/adapter/outbound/app_server/` | `codex app-server` runtime/process/protocol adapters. |
| `src/adapter/outbound/db/` | SQLite authority, task repository, active documents, runtime events, leases, session detail, distributor queue, and repo-scoped workspace persistence. |
| `src/adapter/outbound/filesystem/` | Planning workspace file adapter and scaffold/repair support. |
| `src/adapter/outbound/git/` | Local git and worktree operations for parallel mode. |
| `src/adapter/outbound/github/` | GitHub PR, review, and automation boundary. |
| `src/adapter/outbound/telegram/` | Telegram HTTP API adapter. |
| `src/composition/` | Production dependency graph wiring. |
| `schema/` | Checked-in app-server v2 and server-request protocol snapshots. Refresh both with `bash scripts/refresh_codex_app_server_schema.sh`; provenance records the generator CLI, source artifact, and pristine checksum. |
| `templates/admin/`, `assets/admin/` | Admin templates and embedded visual assets. |
| `assets/app-server/skills/` | Runtime app-server skill assets shipped in release bundles. |
| `npm/` | npm launcher, platform resolver, packaging tests, and publish staging. |
| `scripts/` | PR checks, packaging, validation capture, release verification, planning-tool wrapper, GitHub identity wrapper, and worktree cleanup. |
| `docs/` | Current contracts, design references, operational runbooks, validation records, and agent guidance. |

The pinned `ServerRequest` schema classifies every server-initiated method. Codex 0.144 approval
response shapes are additionally held by exact JSON unit tests until those separately generated
response schema artifacts are pinned alongside the request bundle.

Boundary rules:

- Inbound adapters parse input, render output, and map transport-specific requests.
- `core` owns app lifecycle coordination, not concrete TUI, HTTP, DB, git, or filesystem work.
- Application services own use-case ordering and call ports.
- Domain code owns invariants and pure decisions only.
- Outbound adapters own process, stdio, JSON, SQLite, filesystem, git, GitHub, and Telegram details.
- Mapping logic stays in adapters. Durable policy stays in domain or application services.

See [docs/design/04-hexagonal-runtime-architecture.md](docs/design/04-hexagonal-runtime-architecture.md).

## Development

```bash
. "$HOME/.cargo/env"
cargo build
cargo test
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
```

For the native PR gate used by CI:

```bash
bash scripts/check_native_pr.sh
```

Use focused tests for narrow work:

```bash
cargo test --test architecture_boundaries
cargo test app_server
cargo test planning
```

Preferred coverage areas are startup checks, app-server response parsing, stream reduction, session
list mapping, planning validation, queue projections, admin/API task intake, and parallel
distributor recovery.

## Packaging, Release, And Validation

Build a native archive:

```bash
./scripts/package_native_release.sh --target x86_64-unknown-linux-gnu
```

Verify a bundle and archive:

```bash
./scripts/verify_native_release.sh \
  --archive dist/native/codex-exec-loop-native-<version>-<target>.tar.gz \
  --bundle-dir dist/native/codex-exec-loop-native-<version>-<target> \
  --version <version> \
  --target <target> \
  --profile release
```

Verification requires Node.js 18 or newer; archive policy verification also requires `tar`.

Stable tags using exactly `vMAJOR.MINOR.PATCH` trigger
`.github/workflows/release-native-assets.yml`; prerelease and build-metadata versions fail closed
until they have an explicit npm dist-tag policy. The tag commit must resolve to `GITHUB_SHA` and be
contained in the explicitly fetched `origin/prerelease`. npm publication runs only in the protected
`npm-release` GitHub Environment, whose environment-scoped `NPM_TOKEN` must be a read/write granular
token limited to `@refinedstone/akra`. Configure that environment with required reviewers and tag
deployment rules, and protect `v*` tag create/update/delete with a repository ruleset. Source code
cannot impose those repository settings or repair an older commit's workflow, so they are mandatory
operator controls. The secret is available only to authentication and publish steps after the
environment gate. The workflow builds and verifies Linux, Windows, and macOS
bundles, publishes immutable npm versions through the non-semver `release-staging` dist-tag,
converges `latest` to the registry's highest stable version and `platform` independently, and then
creates or updates the matching GitHub Release. Runs for the same tag are serialized without
cancellation; different tags may finish in either order and re-read registry state around tag
mutation so an older run cannot move a dist-tag backward.

Every npm directory is packed once and that exact tarball is published. A retry skips an existing
immutable npm version only after its registry `dist.integrity` matches the local `npm pack --json`
integrity. Existing GitHub assets are likewise downloaded and compared byte-for-byte; release
tag/title/stable metadata and the exact expected asset-name set are required, and differing or extra
content is never accepted. If npm succeeds but GitHub Release publication fails, repair the release
metadata/assets and rerun the same tag; immutable npm bytes are verified before the release resumes.
Native `.tar.gz` assets are reproducible: the tag commit timestamp is
passed as `SOURCE_DATE_EPOCH`, archive members are sorted with normalized metadata, and gzip omits
variable header timestamps. npm publication generates provenance attestations through the job's
GitHub Actions OIDC identity. Authentication, version lookup, and publication are pinned to
`https://registry.npmjs.org` and ignore registry overrides from the runner environment or user
configuration. Rust is pinned to `1.95.0`, but hosted runner images, system linkers, and platform
SDKs can still drift; a delayed same-tag rebuild with different binary bytes is rejected by the
immutable asset guard and must reuse the original artifact or ship under a new version.

Bundle verification treats release metadata as an exact protocol. `VERSION.txt` must be canonical
UTF-8/LF text with exactly the seven keys `name`, `version`, `release_tag`, `target`, `profile`,
`binary`, and `launcher`, each matching the requested bundle contract. `SHA256SUMS.txt` accepts only
lowercase 64-hex SHA-256 records separated from a safe relative path by exactly two spaces, excludes
itself, rejects duplicate or traversal paths, and covers every other regular bundle file exactly
once. The verifier streams and hashes the archive snapshot, checks bounded safe tar members, then
rehashes the snapshot and every extracted file against the same manifest before npm staging or
publication.

Record terminal validation when a change affects shell rendering, prompt behavior, viewport
handling, scrollback insertion, resize, overlays, status copy, queue/planning surfaces, or
parallel-mode operator flow:

```bash
bash scripts/capture_native_validation.sh \
  --frontend inline \
  --check-profile terminal-baseline \
  --terminal "iTerm2 3.5" \
  --result pass \
  --output-dir docs/validation
```

Summarize recorded validation (informational; warns when required rows are incomplete):

```bash
bash scripts/summarize_native_validation.sh
```

Use explicit gate mode when the validation summary must fail the run:

```bash
bash scripts/summarize_native_validation.sh --fail-on-incomplete
```

Validation references:

- [docs/plan/12-platform-validation-matrix.md](docs/plan/12-platform-validation-matrix.md)
- [docs/validation/README.md](docs/validation/README.md)
- [docs/validation/terminal-ui-testing-methodology.md](docs/validation/terminal-ui-testing-methodology.md)

## Diagnostics And Tracing

Debug and release builds keep file tracing off by default. A valid `AKRA_TRACE` setting is required
before Akra creates trace JSONL under `.codex-exec-loop/runtime/log/`; `RUST_LOG` can only refine
the filter after that explicit opt-in.
Structured trace events replace prompt, response, title/task-title, summary, and error bodies with
character-count metadata at the formatter boundary. Trace files can still contain repository paths
and protocol identifiers, so enable them only for deliberate diagnostics. Daily trace directories
and files are opened through non-symlink, owner-checked boundaries and revalidated on rollover. The
default rolling destination is a private directory with owner-only files; an explicit
`AKRA_TRACE_FILE` still rejects symlinks, hardlinks, and unsafe parents but preserves the operator's
existing parent-directory permissions.

| Setting | Effect |
| --- | --- |
| `AKRA_TRACE=0 cargo run` | Keep file tracing disabled. Empty or invalid values also fail closed to off. |
| `AKRA_TRACE=1 cargo run` | Enable the concise Akra debug preset. |
| `AKRA_TRACE=planning cargo run` | Focus on planning, post-turn evaluation, and planning-worker paths. |
| `AKRA_TRACE=full cargo run` | Enable global trace output and full span lifecycle events. |
| `AKRA_TRACE=1 RUST_LOG=codex_exec_loop_native=trace cargo run` | Refine an enabled trace with standard `tracing_subscriber::EnvFilter` syntax. |
| `AKRA_TRACE_FILE=/tmp/akra-trace.jsonl` | Override the trace JSONL destination. Parent components must not be symbolic links or nonsticky group/world-writable directories. The exact file is opened relative to a pinned parent handle and must be a single-link regular file. Existing parent permissions are preserved. |
| `AKRA_TRACE_MAX_FILES=14` | Retain at most this many rolling trace files (default `7`, maximum `365`). |
| `AKRA_TRACE_MAX_FILE_BYTES=16777216` | Rotate daily trace output at this per-file limit; an exact `AKRA_TRACE_FILE` stops accepting events at the limit (default 16 MiB, minimum 64 KiB, maximum 1 GiB). |
| `AKRA_TRACE_MAX_TOTAL_BYTES=67108864` | Prune the oldest rolling trace segments to this aggregate limit (default 64 MiB, minimum 64 KiB, maximum 4 GiB). Exact-file output uses the lower of this value and the per-file limit. |
| `CODEX_EXEC_LOOP_PLANNER_VISIBILITY=debug cargo run` | Expose full planner prompt/response details in debug-only TUI surfaces. |
| `RUSTFLAGS="--cfg tokio_unstable" AKRA_TOKIO_CONSOLE=1 cargo run --features tokio-console` | Add the tokio-console layer. |

Useful GitHub review/polling variables:

- `CODEX_EXEC_LOOP_GITHUB_PR=owner/repo#123`
- `CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS=60`
- `AKRA_GITHUB_LOGIN=<login>` (required for every GitHub write; repo-local
  `git config akra.githubLogin <login>` is the alternative)
- `AKRA_GITHUB_TOKEN=<token>`
- Frozen parallel delivery accepts the parent token variables above or `gh auth token`; a
  repository/global `credential.helper` alone is intentionally insufficient for autonomous writes.
- `AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN` is no longer supported. Any presence fails closed; direct
  repository, home-directory, or WSL credential-file scanning is never enabled. Use an explicit
  token environment variable or a trusted `gh auth token` session.
- GitHub review activity is bounded by one 90-second aggregate deadline, 20 pages, 2,000 items,
  32 MiB of aggregate decoded JSON, and 8 MiB per HTTP response. Redirects and non-HTTPS API targets
  fail closed, and token-bearing curl input and response files stay outside repository control.

Security-sensitive host tools do not run through an inherited command name. Akra pins native
`git`, `gh`, `curl`, and `bash` executables outside repository/pool-controlled paths, gives
token-bearing GitHub and Telegram subprocesses a minimal scrubbed environment, and fails closed on
a hostile or relative `PATH`. The reviewed `gh-akra` helper is embedded in the binary and passed to
the pinned `bash --noprofile --norc -s --` over bounded stdin instead of reopening the tracked
repository script. Frozen remote operations additionally use an owner-private isolated Git
configuration/object context and accept only credential-free HTTPS routing plus validated proxy/CA
inputs; repository/global credential helpers and transport rewrites are not imported.

Native pinning parses the supported host format from one identity-checked file handle: ELF64 on
Linux, thin or fat Mach-O 64 on macOS, and PE32+ on Windows. The architecture must match the running
Akra target, format tables stay inside the file and a 16 MiB metadata budget, and the declared entry
point must resolve into an executable load segment or section. Path identity is checked again after
inspection; the same-user process limitation below still applies after that handle is released.

## Current Limits

- The counted `terminal-baseline` release gate remains `0/4` required passes. Documentation and
  automated rendering tests do not replace captures from macOS Terminal.app, iTerm2, Windows
  Terminal PowerShell, and Windows Terminal WSL bash.
- Real-terminal validation is still required after changes to prompt editing, streaming, overlays,
  terminal restore, restart recovery, blocked distributor flow, or multi-worktree operation.
- Native Windows and macOS host validation remains distinct from Linux checks and pure parser/unit
  tests. In particular, Windows ACL/standard npm `.cmd` handling and the platform pool lock need
  supported-host execution evidence; cross-compilation alone must not be reported as that evidence.
- Planning detail mode supports manual authoring only; `llm-assisted` planning authoring is
  disabled.
- Interactive approval currently covers command execution and validated additional permissions
  only. Permission grants are constrained to the current turn. File-change approval requests are
  fail-closed because their complete change and grant scope cannot be inspected. Hidden planning and
  parallel workers decline automatically, and timeout, interrupt, disconnect, or invalid payloads
  also decline. Tool user-input and MCP elicitation requests receive schema-shaped cancel/decline
  responses; dynamic tool, auth-token refresh, and attestation server requests remain unsupported.
- Non-git workspaces do not use the full supersession worktree pool model.
- Windows does not expose direct planning filesystem or external file-sync workflows until every
  component can be opened relative to a pinned NT directory handle. Git-backed and non-Git private
  SQLite authority plus the admin draft editor remain supported.
- Windows parallel workers use the built-in validated agent profiles. Custom workspace profile
  files stay unread and read-only until they can be opened relative to a pinned NT directory
  handle, so profile customization is unavailable while worker dispatch remains supported. Pool
  lease, distributor, and session JSON mirrors are likewise omitted on Windows; SQLite remains the
  runtime source of truth.
- File permissions, owner checks, pinned handles, and before/after identity validation reduce
  cross-user and accidental races; they do not isolate Akra from a malicious process already running
  as the same OS user. Use a separate account, sandbox, or VM for that threat model.
- Linux descendant cleanup follows a random inherited marker and process-start identities, but a
  descendant that deliberately clears the marker and fully daemonizes after every observable parent
  exits still requires a delegated cgroup, PID namespace, or VM for a kernel-enforced guarantee.
  macOS and other non-Linux Unix targets provide process-group containment only, so a descendant
  that calls `setsid` can escape that group. Unix TUI, admin, and Telegram processes convert
  SIGINT/SIGTERM/SIGHUP into graceful shutdown, and the npm wrapper performs a bounded descendant
  ancestry/process-group sweep before forced termination. SIGKILL against the wrapper itself still
  bypasses user-space cleanup.
- `AKRA_APP_SERVER_PROCESS_ENVIRONMENT=all` is an explicit compatibility escape hatch, not a secure
  mode: it exposes every parent credential to app-server and same-user process inspection.
- Release archive file names use the package version declared in `Cargo.toml`; the release tag must
  use stable `vMAJOR.MINOR.PATCH`, npm publish staging uses that tag version without the leading
  `v`, and an official tag release fails closed unless `NPM_TOKEN` is present and authenticated.
- GitHub tag-target checks run immediately before and after release mutations, but only a protected
  server-side `v*` tag ruleset closes the small check-to-mutation race. npm versions are immutable:
  once publication succeeds it cannot be rolled back, so a later GitHub Release failure must be
  repaired and the same tag rerun against the verified npm bytes.
- Public GitHub/npm adoption metrics are time-sensitive publication inputs, not repository truth.
  Refresh and date them immediately before external use.

## Documentation Index

- [AGENTS.md](AGENTS.md): authoritative repository workflow and architecture rules for coding
  agents.
- [.gemini/styleguide.md](.gemini/styleguide.md): Gemini entrypoint that delegates to the same
  worktree, delivery, review-language, and Rust toolchain contract.
- [docs/README.md](docs/README.md): compact map of current docs.
- [docs/supersession/current-contract.md](docs/supersession/current-contract.md): shipped planning,
  continuation, and parallel-mode operator contract.
- [docs/design/01-current-product-state.md](docs/design/01-current-product-state.md): product
  identity, surface map, runtime shape, and code entry.
- [docs/design/02-tui-shell-flow.md](docs/design/02-tui-shell-flow.md): operator-visible shell
  modes and flow.
- [docs/design/04-hexagonal-runtime-architecture.md](docs/design/04-hexagonal-runtime-architecture.md):
  layer ownership and architecture gates.
- [docs/design/05-parallel-control-plane-architecture.md](docs/design/05-parallel-control-plane-architecture.md):
  parallel-mode ownership and control-plane rules.
- [docs/design/06-planning-runtime-and-draft-editor.md](docs/design/06-planning-runtime-and-draft-editor.md):
  planning authority, staged drafts, runtime task intake, and recovery.
- [docs/design/08-parallel-mode-supersession-board.md](docs/design/08-parallel-mode-supersession-board.md):
  shipped parallel board shape.
- [docs/plan/13-native-packaging-and-operator-runbook.md](docs/plan/13-native-packaging-and-operator-runbook.md):
  native bundle, npm, release, and operator handoff.
- [docs/plan/14-codex-for-oss-application.md](docs/plan/14-codex-for-oss-application.md):
  Codex for Open Source application positioning and form-answer draft.
