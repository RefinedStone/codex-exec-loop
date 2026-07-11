# Agent Canvas Evidence Ledger

This ledger separates inspected source, reproduced release-gate output, vendor documentation,
inference, and unverified experiments. Product conclusions are in [analysis.md](analysis.md), and
Akra work decisions are in [gap-matrix.md](gap-matrix.md).

## Snapshot

| Field | Value |
| --- | --- |
| Product | OpenHands Agent Canvas |
| Official repository | <https://github.com/OpenHands/agent-canvas> |
| Release | [v1.2.1](https://github.com/OpenHands/agent-canvas/releases/tag/v1.2.1) |
| Release commit | `56d51c0767fb6fedc51c466f5138fdfc116a2707` |
| Release commit date | 2026-07-10 17:52:12 +07:00 |
| Audit date | 2026-07-12 (Asia/Seoul) |
| Akra baseline | `0229ed71a541d91abc04ad13b188d28b9eafcaf8` on `prerelease` |
| Akra version | 1.3.5 |
| Source-audit environment | Linux x86_64, clean detached source checkouts |
| Release-gate environment | Ubuntu 24.04.2 under WSL2, x86_64, Node 24.14.1, npm 11.6.1 |

The Agent Canvas and dependency checkouts were public, locally available, clean detached
checkouts. The Akra baseline was a clean worktree from `origin/prerelease`.

## Evidence Classes

- `verified`: inspected at an immutable source commit, observed in release metadata, or reproduced
  locally;
- `documented`: stated in official project documentation but not reproduced in a running product;
- `proposed`: explicitly future, beta, experimental, or present only in a later dependency path;
- `inferred`: a conclusion from verified facts whose runtime consequence was not reproduced;
- `unverified`: a claim or comparison missing the required experiment.

Inventory counts are orientation data, not quality scores. Build duration and maximum RSS describe
the command that ran, not interactive product performance.

## Pinned Source Set

| Component | Tag or snapshot | Commit | Role |
| --- | --- | --- | --- |
| Agent Canvas | v1.2.1 | `56d51c0767fb6fedc51c466f5138fdfc116a2707` | released browser product |
| OpenHands software-agent-sdk | v1.33.0 | `7ef44d10b2132125833ce2269559b3960f402805` | released Agent Server/ACP runtime |
| OpenHands Automation | 1.1.4 | `7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71` | released Automation service |
| Zed Codex ACP | v0.16.0 | `bb590500e8646f6daf879b8b3c6a659fbd29017d` | released embedded-Codex ACP bridge |
| OpenHands software-agent-sdk | v1.35.0 | `9028562e2d5eda76de662ec9b7584125760eb83f` | later path, not shipped by Canvas v1.2.1 |
| Agent Client Protocol Codex ACP | v1.1.2 | `8aff492d4b033ff2c02ad3b9d591994d57617463` | later app-server bridge |

The Canvas `main` branch was `90223f99f43a977ec454c0fc15e93642b7053075` and the SDK `main`
branch was `cf6c2a3a4ace65b651ea29032ab6b4a74d7bb41a` when inspected. Those moving tips are recorded only
as refresh hints; no shipped conclusion is based on them.

## Reproduction Commands

### Source Checkouts

```bash
git clone --depth 1 --branch v1.2.1 \
  https://github.com/OpenHands/agent-canvas.git /tmp/akra-agent-canvas-v121-audit
git -C /tmp/akra-agent-canvas-v121-audit rev-parse HEAD
git -C /tmp/akra-agent-canvas-v121-audit describe --tags --exact-match
git -C /tmp/akra-agent-canvas-v121-audit log -1 --format='%cI %s'
git -C /tmp/akra-agent-canvas-v121-audit status --short

git clone --depth 1 --branch v1.33.0 \
  https://github.com/OpenHands/software-agent-sdk.git \
  /tmp/akra-software-agent-sdk-v133-audit
git clone --depth 1 --branch v1.35.0 \
  https://github.com/OpenHands/software-agent-sdk.git \
  /tmp/akra-software-agent-sdk-v135-audit
git clone --depth 1 --branch 1.1.4 \
  https://github.com/OpenHands/automation.git /tmp/akra-openhands-automation-114-audit
git clone --depth 1 --branch v0.16.0 \
  https://github.com/zed-industries/codex-acp.git /tmp/akra-zed-codex-acp-v016-audit
git clone --depth 1 --branch v1.1.2 \
  https://github.com/agentclientprotocol/codex-acp.git /tmp/akra-codex-acp-v112-audit
```

Observed Canvas identity:

```text
56d51c0767fb6fedc51c466f5138fdfc116a2707
v1.2.1
2026-07-10T17:52:12+07:00 chore(main): release 1.2.1 (#1646)
```

### Inventory

```bash
CANVAS=/tmp/akra-agent-canvas-v121-audit
cd "$CANVAS"

git ls-files | wc -l
git ls-files '*.ts' '*.tsx' | wc -l
git ls-files '*.ts' '*.tsx' | xargs wc -l | tail -1
git ls-files '*.ts' '*.tsx' '*.js' '*.mjs' '*.css' | wc -l
git ls-files '*.ts' '*.tsx' '*.js' '*.mjs' '*.css' \
  | xargs wc -l | tail -1
git ls-files '*.test.ts' '*.test.tsx' '*.spec.ts' '*.spec.tsx' | wc -l
git ls-files | rg '^tests/e2e/.+\.spec\.(ts|tsx)$' | wc -l
rg -n '\b(it|test|describe)\s*\(' \
  --glob '*.ts' --glob '*.tsx' --glob '*.js' --glob '*.mjs' | wc -l
```

Observed:

| Inventory | Count |
| --- | ---: |
| Tracked files | 1,780 |
| Tracked TypeScript files (`.ts`, `.tsx`) | 1,525 |
| Tracked TypeScript LOC | 197,327 |
| Tracked TS/TSX/JS/MJS/CSS files | 1,553 |
| Tracked TS/TSX/JS/MJS/CSS LOC | 206,587 |
| Test/spec files by TypeScript pathname | 500 |
| E2E spec files | 19 |
| Repository-wide `it`/`test`/`describe` call markers in the listed JS-family files | 4,618 |

The marker count is a text-search result, not an executed-test count. Tests can be parameterized,
skipped, generated, or named in ways the expression does not count.

### Release Gates

The release gates ran in a separate exported source tree without `.git` metadata at
`/tmp/agent-canvas-v121-run`:

```bash
cd /tmp/agent-canvas-v121-run
/usr/bin/time -v npm ci
/usr/bin/time -v npm run lint
/usr/bin/time -v npm test
/usr/bin/time -v npm run build
/usr/bin/time -v npm run build:lib
npm pack --dry-run --json
npm audit --json
npm audit --omit=dev --json
```

Observed on 2026-07-12:

| Gate | Result |
| --- | --- |
| `npm ci` | passed; installed 1,301 packages |
| `npm run lint` | passed in 46.62 s; maximum RSS 3,362,648 KiB |
| `npm test` | passed in 52.82 s; maximum RSS 763,632 KiB |
| Vitest result | 480 files passed, 1 skipped; 3,666 tests passed, 5 skipped, 9 todo |
| `npm run build` | passed in 8.76 s; maximum RSS 1,759,796 KiB; 6,897 client modules |
| `npm run build:lib` | passed in 12.83 s; maximum RSS 1,581,092 KiB |
| `npm pack --dry-run --json` | passed; 14,215,651-byte tarball, 57,069,667 unpacked bytes, 9,439 entries |
| `npm audit --json` | exit 1; 2 low, 11 moderate, 5 high advisories |
| `npm audit --omit=dev --json` | exit 1; 2 low, 11 moderate, 4 high advisories |

Vitest warned about nested `vi.mock`/`vi.hoisted` behavior that a future Vitest release will reject
and missing source-map sources in a generated TypeScript client. The application build warned
about browser externalization of `node:http`/`node:https`, React Router future flags, and chunks
larger than 500 kB after minification. The largest recorded client chunk was 526.28 kB before gzip;
the library translation bundle was 1,303.12 kB before gzip.

The advisory totals are a lockfile audit at one date. Reachability and exploitability were not
tested. The pack prepare step reported that `.git` could not be found because the validation tree
was an export, but still exited successfully.

## Immutable Source Ledger

### Product, Packaging, And Boundary

- [README product thesis](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/README.md#L1-L45)
  names the control-center role, supported agent families, local/remote/cloud targets, Beta status,
  and deployment choices. Class: `documented`.
- [Architecture boundary](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/docs/architecture.md#L1-L31)
  assigns runtime, sandbox, workspace, and event history to Agent Server rather than Canvas. Class:
  `verified` for the source boundary.
- [Pinned defaults](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/config/defaults.json#L1-L14)
  bind Canvas 1.2.1 to Agent Server 1.33.0, Automation 1.1.4, and Python ACP below 0.11. Class:
  `verified`.
- [Package scripts](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/package.json#L74-L103)
  omit `dev:docker`, `dev:dangerously-dockerless`, and `dev:automation`, although the
  [architecture guide](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/docs/architecture.md#L45-L56)
  names them. Class: `verified` documentation/source drift.
- [All-in-one image services](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/docker/Dockerfile#L3-L15)
  and [entrypoint startup](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/docker/entrypoint.sh#L229-L263)
  verify the static proxy, Agent Server, and optional Automation process topology. Class:
  `verified`.

### Conversation Creation, Reconnect, And Workspaces

- [Home launcher state and submission](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/home/home-chat-launcher.tsx#L37-L115)
  initialize the attached-local-workspace selector to `local_repo`, while start-from-scratch omits
  workspace and mode. Class: `verified`.
- [Conversation creation](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/api/conversation-service/agent-server-conversation-service.api.ts#L394-L439)
  resolves an attached workspace without an explicit override to `local_repo`, but resolves omitted
  workspace and mode to `new_worktree`; `worktree` follows that resolved value. Class: `verified`.
- [Agent Server worktree creation](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-agent-server/openhands/agent_server/conversation_service.py#L62-L246)
  creates `/tmp/conversation-worktrees/<conversation-id>/`, an `openhands/<uuid>` branch, and
  system guidance. Class: `verified`.
- [Agent Server deletion](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-agent-server/openhands/agent_server/conversation_service.py#L904-L943)
  removes conversation state but explicitly preserves workspace data. No worktree cleanup path was
  found by the audit's source search. Class: `verified` for preservation, `inferred` for the absence
  of cleanup beyond the searched snapshot.
- [Client conversation metadata](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/api/conversation-metadata-store.ts#L4-L84)
  persists selected repository, branch, workspace, and workspace mode in browser local storage.
  The [Agent Server adapter](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/api/conversation-service/agent-server-conversation-service.api.ts#L404-L439)
  explicitly says the runtime has no corresponding selected-source concept. Class: `verified`.
- [Conversation WebSocket context](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/contexts/conversation-websocket-context.tsx#L115-L174)
  shows event sequence tracking and a single planning-agent assumption for subconversations. Class:
  `verified`.
- [WebSocket reconnect](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/hooks/use-websocket.ts#L36-L154)
  uses a fixed retry delay and reconnect loop. Class: `verified`.
- [Conversation-list query](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/hooks/query/use-paginated-conversations.ts#L8-L47)
  polls every ten seconds. Class: `verified`.

### Selected-Conversation UX

- [Conversation split](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/conversation/conversation-main/conversation-main.tsx#L21-L126)
  and [inspector tabs](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/conversation/conversation-tabs/conversation-tabs.tsx#L80-L145)
  verify the chat, files, planner, terminal, browser, and task surfaces. Class: `verified`.
- [Terminal hook](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/hooks/use-terminal.ts#L88-L190)
  creates xterm with input disabled and writes observed logs. Class: `verified`.
- [Browser panel](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/browser/browser.tsx#L6-L25)
  renders a screenshot, URL, and external-open control rather than an interactive embedded browser.
  Class: `verified`.
- [File viewer](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/files-tab/file-content-viewer.tsx#L104-L227),
  [2,000-file query bound](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/hooks/query/use-workspace-files.ts#L7-L34),
  and [100-file change bound](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/routes/changes-tab.tsx#L49-L119)
  define the read-only and bounded file/diff surface. Class: `verified`.
- [Conversation rail](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/conversation-panel/conversation-panel.tsx#L220-L289)
  verifies pin, group, filter, sort, and active-session navigation. Class: `verified`.
- [Status-dot mapping](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/conversation-panel/conversation-status-dot.tsx#L24-L59)
  gives idle and approval-waiting sessions the same green working presentation. Class: `verified`.
- [Event reduction](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/utils/handle-event-for-ui.ts#L147-L368)
  and [confirmation controls](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/shared/buttons/conversation-confirmation-buttons.tsx#L16-L132)
  verify broad OpenHands event and approval rendering outside the ACP loss described below. Class:
  `verified`.
- [Goal interceptor](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/hooks/chat/use-goal-interceptor.ts#L9-L64)
  and [goal status](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/chat/goal-status-content.tsx#L22-L168)
  verify the Agent Server judge-loop controls and projection. Class: `verified` for UI, not for
  judge quality.
- [Git action helpers](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/utils/utils.ts#L458-L500)
  return natural-language pull, push, branch, and create-PR prompts. The
  [Git tools menu](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/controls/git-tools-submenu.tsx#L28-L50)
  inserts those strings into the message-to-send state. Class: `verified` prompt-only control.

### Released Codex ACP Path

- Canvas documents the
  [ACP subprocess relay](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/docs/ACP_AGENTS.md#L8-L31).
  The pinned SDK registry selects
  [`@zed-industries/codex-acp@0.16.0`](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-sdk/openhands/sdk/settings/acp_providers.py#L372-L440).
  Class: `verified`.
- The released bridge's
  [Cargo manifest](https://github.com/zed-industries/codex-acp/blob/bb590500e8646f6daf879b8b3c6a659fbd29017d/Cargo.toml#L20-L37)
  pins ACP and Codex `rust-v0.137.0` crates, and its
  [entry point](https://github.com/zed-industries/codex-acp/blob/bb590500e8646f6daf879b8b3c6a659fbd29017d/src/lib.rs#L15-L70)
  serves ACP with embedded Codex state. No app-server dependency or child exists. Class: `verified`.
- [ACP settings resolution](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-sdk/openhands/sdk/settings/model.py#L1650-L1743)
  turns a built-in preset with no explicit command into the pinned registry command and can rewrite
  it to a preinstalled binary. Class: `verified`.
- [SDK event bridge](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-sdk/openhands/sdk/agent/acp_agent.py#L1200-L1318)
  handles text, thought, usage, and tool updates but not ACP Plan updates; tool progress is condensed
  before persistence. Class: `verified`.
- The old bridge emits
  [ACP Plan updates](https://github.com/zed-industries/codex-acp/blob/bb590500e8646f6daf879b8b3c6a659fbd29017d/src/thread.rs#L2708-L2723),
  but the SDK event bridge above does not consume them. Plan events are therefore lost end to end in
  the released path. Separately, Canvas
  [hides the Code/Plan mode switch](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/chat/components/chat-input-actions.tsx#L81-L84)
  for ACP sessions. Class: `verified`.
- [Permission callback](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-sdk/openhands/sdk/agent/acp_agent.py#L1372-L1389)
  selects the first option without user interaction. Class: `verified`.
- The released bridge's generic permission list puts
  [session-wide approval first](https://github.com/zed-industries/codex-acp/blob/bb590500e8646f6daf879b8b3c6a659fbd29017d/src/thread.rs#L2339-L2347).
  The later maintained bridge uses the
  [same ordering](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/src/CodexApprovalHandler.ts#L172-L190).
  Command/file requests can have a different first option. Class: `verified`.
- [Default Canvas confirmation settings](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/services/settings.ts#L5-L62)
  disable confirmation. The Canvas adapter maps that setting to
  [`NeverConfirm`](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/api/agent-server-adapter.ts#L477-L489)
  in the outer OpenHands
  [conversation request](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/api/agent-server-adapter.ts#L930-L954),
  while the SDK provider registry uses full-access mode. Neither setting routes ACP
  `request_permission` to an operator; that callback is the separate bridge path above. Class:
  `verified`.
- [ACP card content](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/conversation-events/chat/event-content-helpers/get-acp-tool-call-content.ts#L70-L131)
  reads raw input/output rather than the wrapper's structured diff content. Class: `verified`.
- [ACP isolation limitation](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/docs/ACP_AGENTS.md#L204-L212)
  says same-provider conversations can share HOME state because the released TypeScript client
  cannot send `acp_isolate_data_dir`. Ambient-login provider config, cache, and lock files can
  therefore be shared. Separately, the SDK
  [materializes registered file credentials](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-sdk/openhands/sdk/agent/acp_agent.py#L2149-L2224)
  such as `CODEX_AUTH_JSON` into per-conversation storage and points `CODEX_HOME` there. Class:
  `documented` for the Canvas limit and `verified` for the SDK exception.
- [ACP child environment](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-sdk/openhands/sdk/agent/acp_agent.py#L2300-L2347)
  starts from the full parent environment, strips inherited npm and selected conflict variables,
  and adds all non-file values from the secret registry. Its
  [file-secret permissions](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-sdk/openhands/sdk/agent/acp_agent.py#L2214-L2277)
  use restricted directory and file modes. Class: `verified`.
- [Resume fallback](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-sdk/openhands/sdk/agent/acp_agent.py#L2516-L2581)
  can create a fresh session after a load protocol error. Class: `verified`; silent-continuity impact
  is `inferred` pending a fault test.
- [Prompt retry](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-sdk/openhands/sdk/agent/acp_agent.py#L3400-L3479)
  retries selected transport and internal failures. Duplicate accepted turns are an `unverified`
  risk until response-loss injection is run.
- The released bridge advertises only
  [close, list, and resume session capabilities](https://github.com/zed-industries/codex-acp/blob/bb590500e8646f6daf879b8b3c6a659fbd29017d/src/codex_agent.rs#L452-L461),
  and its [registered request handlers](https://github.com/zed-industries/codex-acp/blob/bb590500e8646f6daf879b8b3c6a659fbd29017d/src/codex_agent.rs#L120-L307)
  have no fork method. Class: `verified` fork absence.

### Later Codex ACP Path, Not Shipped

- SDK 1.35 switches its registry to
  [`@agentclientprotocol/codex-acp@1.1.2`](https://github.com/OpenHands/software-agent-sdk/blob/9028562e2d5eda76de662ec9b7584125760eb83f/openhands-sdk/openhands/sdk/settings/acp_providers.py#L378-L446)
  and its [Dockerfile](https://github.com/OpenHands/software-agent-sdk/blob/9028562e2d5eda76de662ec9b7584125760eb83f/openhands-agent-server/openhands/agent_server/docker/Dockerfile#L176-L195)
  installs the same version. Class: `proposed` relative to Canvas v1.2.1.
- The maintained bridge
  [spawns `codex app-server`](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/src/CodexJsonRpcConnection.ts#L15-L42)
  and its [package manifest](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/package.json#L63-L69)
  declares Codex `^0.144.0`. Class: `verified` in that later component, `proposed` for Canvas.
- Its event handler emits
  [Plan](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/src/CodexEventHandler.ts#L551-L561)
  but ignores native diff/patch notifications in its
  [event switch](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/src/CodexEventHandler.ts#L182-L221).
  SDK 1.35's [Python event bridge](https://github.com/OpenHands/software-agent-sdk/blob/9028562e2d5eda76de662ec9b7584125760eb83f/openhands-sdk/openhands/sdk/agent/acp_agent.py#L1221-L1339)
  still does not consume Plan. Class: `verified` in the later source path.
- The maintained bridge's
  [capabilities](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/src/CodexAcpServer.ts#L207-L229)
  and [registered handlers](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/src/index.ts#L106-L132)
  omit fork, while SDK 1.35
  [`ask_agent()`](https://github.com/OpenHands/software-agent-sdk/blob/9028562e2d5eda76de662ec9b7584125760eb83f/openhands-sdk/openhands/sdk/agent/acp_agent.py#L3566-L3617)
  calls `fork_session`. A method-not-found result is `inferred`; it was not reproduced end to end.
- [Optional app-server logging](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/src/CodexJsonRpcConnection.ts#L45-L60)
  records raw frames, and [prompt logging](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/src/CodexAcpServer.ts#L1425-L1429)
  can include raw prompt data. Leakage is an `inferred` opt-in risk pending a canary test.
- [Workspace trust injection](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/src/CodexAcpClient.ts#L379-L390)
  marks configured roots trusted. Class: `verified` in the later bridge.

### Backend Registry And Telemetry

- [Backend record](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/api/backend-registry/types.ts#L1-L9)
  includes host and API key, and
  [storage](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/api/backend-registry/storage.ts#L80-L120)
  persists the record in browser local storage. Class: `verified`.
- [Backend health query](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/hooks/query/use-backends-health.ts#L63-L299)
  verifies health degradation and recovery handling. Class: `verified`.
- [Anonymous install telemetry](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/hooks/use-telemetry.ts#L29-L80)
  can send one install event before normal tracking consent unless DNT or environment policy disables
  it. Class: `verified`.

### Automation 1.1.4

- [Run model](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/models.py#L121-L200)
  stores status, error, conversation, sandbox, command, payload, and timestamps. Class: `verified`.
- [Key/value encryption](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/utils/kv.py#L175-L218)
  validates, encrypts, and decrypts per-automation JSON state through the SDK Fernet helper. Class:
  `verified`.
- [Automation creation](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/automations/create-instructions.tsx#L52-L95)
  launches a translated prompt in a conversation rather than submitting a deterministic creation
  form. Recommended recipes use the same
  [agent-mediated prompt path](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/automations/recommended-automations-launcher.tsx#L82-L121).
  Class: `verified`.
- The edit modal exposes schedule controls but renders
  [event-trigger fields read-only](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/automations/detail/edit-automation-modal.tsx#L316-L428).
  Class: `verified` partial edit surface.
- [Scheduler](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/scheduler.py#L144-L214),
  [dispatcher](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/dispatcher.py#L347-L400),
  and [watchdog](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/watchdog.py#L225-L328)
  verify durable polling, background dispatch, timeouts, and stale-run reconciliation. Class:
  `verified`.
- [Webhook security note](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/event_router.py#L1-L29)
  records HMAC support and missing replay/rate-limit controls; matching deliveries create new pending
  runs without a source-event deduplication key. Class: `verified`.
- The backend exposes six
  [run states](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/schemas.py#L200-L208),
  but Canvas declares only four in its
  [run type](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/types/automation.ts#L46-L68).
  The [API passes status through](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/api/automation-service/automation-service.api.ts#L153-L174),
  and the [badge dereferences its config](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/automations/detail/run-status-badge.tsx#L13-L72)
  without a fallback. Class: `verified` contract mismatch; runtime `TypeError` is `inferred`.
- The backend's
  [cloud concurrency path](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/dispatcher.py#L197-L208)
  produces `SKIPPED`, and its
  [cancel endpoint](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/router.py#L431-L522)
  produces `CANCELLED`. These are reachable backend states rather than dead enum values. Class:
  `verified`.
- The backend response also carries
  [`timeout_at`, `sandbox_id`, and `created_at`](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/schemas.py#L625-L640),
  which the Canvas run type omits. The Canvas
  [Automation service methods](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/api/automation-service/automation-service.api.ts#L56-L189)
  expose list, update, delete, dispatch, and run listing but no run-cancel call. Class: `verified`.
- [Badge tests](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/__tests__/components/automations/detail/run-status-badge.test.tsx#L7-L31)
  cover only the four Canvas states. Class: `verified` missing contract coverage.

### Quality And Validation

- [Main CI](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/.github/workflows/ci.yml#L23-L99)
  runs install and application build on Ubuntu and Windows. Lint, tests, library build, and package
  verification run only in the Ubuntu `full_checks` job. Class: `verified`.
- [Coverage configuration](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/vite.config.ts#L417-L438)
  exists, but this audit found no threshold or main-CI coverage gate. Class: `verified` for config,
  `inferred` for repository-wide absence after source search.
- Live E2E workflows are manual or label-gated for eligible same-repository pull requests. Mock-LLM
  and Docker E2E have separate workflows. Class: `verified` by workflow inspection; they were not
  executed in this audit.

## Akra Baseline Evidence

- [App-server adapter](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/app_server/mod.rs#L380-L395)
  and [session catalog](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/app_server/mod.rs#L1135-L1175)
  verify direct official app-server initialization, thread operations, and session mapping. Class:
  `verified`.
- [Protocol classification test](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/app_server/protocol/contract_tests.rs#L314-L331)
  fails when an app-server notification has no explicit disposition. Class: `verified`.
- Akra's current approval domain exposes only
  [accept and decline](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/domain/conversation.rs#L245-L262),
  requires app-server command requests to offer both one-turn
  [accept and decline](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/app_server/approval.rs#L471-L490),
  classifies file-change approval as
  [uninspectable](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/app_server/approval.rs#L17-L23)
  and [declines it](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/app_server/connection.rs#L1847-L1858),
  and [declines MCP elicitation](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/app_server/connection.rs#L1870-L1884)
  because it has no typed forms for those paths. The direct boundary is structural; full
  decision-option fidelity is not shipped. Class: `verified`.
- [Child environment policy](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/app_server/connection.rs#L443-L520)
  filters environment variables, and
  [credential forwarding](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/app_server/connection.rs#L725-L773)
  requires explicit API-key opt-in. Class: `verified`.
- [Parallel roster](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/domain/parallel_mode/agent_session.rs#L14-L31)
  and [detail](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/domain/parallel_mode/agent_session.rs#L95-L121)
  verify agent, task, slot, branch, lifecycle, validation, authority, and distributor projection.
  Class: `verified`.
- [Parallel event invalidation](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/application/service/parallel_mode/turn.rs#L348-L358)
  deliberately excludes tool activity and delta/completion events. Class: `verified` current-activity
  gap.
- Current conversation activity has only completed
  [file-change and command-execution kinds](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/domain/conversation.rs#L192-L210),
  and the [provider stream channel](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/application/service/conversation_runtime_event.rs#L1-L20)
  is a bounded synchronous channel with capacity eight. The authority store assigns
  [runtime-event sequence](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/db/sqlite_planning_authority_adapter/runtime_projection.rs#L2564-L2603)
  transactionally. Class: `verified`.
- [Current delivery contract](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/docs/supersession/current-contract.md#L124-L164)
  describes the lease, frozen range, default reviewed PR, explicit high-risk exceptions, serialized
  integration, remote verification, and cleanup flow. Source services were inspected to avoid
  treating the document alone as proof. Class: `verified` for the current contract.
- [Native PR script](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/scripts/check_native_pr.sh#L1-L24)
  runs TUI layering, Node surface, Rust format, test, and clippy gates. The
  [native PR workflow](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/.github/workflows/native-pr-checks.yml#L33-L176)
  adds platform-targeted test jobs and the broad script gate. Class: `verified`.
- [Admin loopback bind](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/inbound/admin_api/mod.rs#L75-L125)
  verifies that Akra does not yet have Canvas-style remote backend switching. Class: `verified`.
- [CLI dispatch](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/inbound/cli.rs#L41-L150)
  exposes Admin, Telegram, planning-tool, and parallel-tick entry points. A repository-wide source
  search found no schedule trigger, signed webhook intake, or durable automation run ledger. Class:
  `verified` for current CLI commands and `inferred` for repository-wide absence after source search.
- [Diorama unit assignment](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/assets/admin/game/src/akra-diorama.ts#L611-L682),
  [continuous movement](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/assets/admin/game/src/akra-diorama.ts#L757-L780),
  and [continuous packet animation](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/assets/admin/game/src/akra-diorama.ts#L836-L876)
  are not derived from durable work events. Class: `verified` truthfulness gap.
- [Visual test](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/scripts/capture_admin_graphic.mjs#L162-L178)
  currently requires canvas pixels to change while idle. Class: `verified`.

## Audit Limits

This audit did not:

- run an authenticated Agent Server plus Codex conversation;
- reproduce the Automation `CANCELLED`/`SKIPPED` badge crash in a browser;
- run three concurrent conversations or provoke the shared-HOME ACP race;
- execute remote/cloud backend failover or a public self-host deployment;
- measure cold start, interactive latency, steady-state memory, or concurrent-session scaling;
- inject ACP permission choices, response loss, process crashes, large output, or version drift;
- verify whether all `npm audit` advisories are reachable in production;
- compare the quality of OpenHands goal judging with Akra's planning authority;
- treat later SDK 1.35 or maintained Codex ACP behavior as shipped Canvas v1.2.1 behavior.

The highest-value missing experiment is a golden app-server trace that measures field preservation,
ordering, identity, permission outcome, and failure recovery through Akra and the maintained ACP
stack. The next is a same-machine authenticated topology benchmark with raw samples and complete
process-tree accounting. Until those exist, protocol and performance claims remain bounded to the
source facts above.
