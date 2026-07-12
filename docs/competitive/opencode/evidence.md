# OpenCode Evidence Ledger

This ledger separates immutable source inspection, reproduced release/runtime output, documented
behavior, inference, and unverified experiments. Product conclusions are in
[analysis.md](analysis.md), and Akra work decisions are in [gap-matrix.md](gap-matrix.md).

## Snapshot

| Field | Value |
| --- | --- |
| Product | OpenCode |
| Official repository | <https://github.com/anomalyco/opencode> |
| Release | [v1.17.18](https://github.com/anomalyco/opencode/releases/tag/v1.17.18) |
| Release commit | `b1fc8113948b518835c2a39ece49553cffe9b30c` |
| Release commit date | 2026-07-09 18:51:29 +00:00 |
| Release publication | 2026-07-09 18:51:45 +00:00 |
| Audit date | 2026-07-12 (Asia/Seoul) |
| Akra baseline | `354f4782a73ffabab6abf342cb0f37285a96c177` on `prerelease` |
| Akra version | 1.3.5 |
| Environment | Ubuntu 24.04.2 LTS under WSL2, Linux 6.18.33.2, x86_64, Bun 1.3.14 |

The OpenCode source checkout was public, locally available, clean, detached at the release tag, and
left without source edits. The Akra baseline was a clean worktree from `origin/prerelease`.

The GitHub release API identified release `351708464` as immutable, non-draft, and non-prerelease.
The Linux x64 archive checksum matched the release asset digest exactly:

```text
e149d32ee5667c0cd5fb84d0bf8393b312e93782eeb4d74d29bbb0392de7133c
```

## Evidence Classes

- `verified`: inspected at the immutable commit, observed in immutable release metadata/artifacts,
  or reproduced locally;
- `documented`: explicitly stated by official project documentation but not reproduced;
- `proposed`: marked experimental, beta, transitional, incomplete, or present outside the released
  default path;
- `inferred`: a bounded consequence reasoned from verified facts but not reproduced end to end;
- `unverified`: missing the required environment, authenticated run, raw samples, or external
  policy evidence.

Inventory counts and bundle sizes are orientation evidence, not quality or runtime-performance
scores. A source route proves a reachable implementation path only under its stated runtime
preconditions.

## Reproduction Commands

### Source And Release Identity

```bash
git clone https://github.com/anomalyco/opencode.git /tmp/akra-opencode-v1.17.18-source
git -C /tmp/akra-opencode-v1.17.18-source checkout --detach v1.17.18
git -C /tmp/akra-opencode-v1.17.18-source rev-parse HEAD
git -C /tmp/akra-opencode-v1.17.18-source describe --tags --exact-match
git -C /tmp/akra-opencode-v1.17.18-source log -1 --format='%cI %s'
git -C /tmp/akra-opencode-v1.17.18-source status --short

curl -fsSL https://api.github.com/repos/anomalyco/opencode/releases/tags/v1.17.18 \
  | jq '{id,tag_name,target_commitish,draft,prerelease,immutable,published_at}'
curl -fsSL https://api.github.com/repos/anomalyco/opencode/releases/tags/v1.17.18 \
  | jq -r '.assets[] | select(.name == "opencode-linux-x64.tar.gz")
      | [.name,.size,.digest] | @tsv'
```

Observed source identity:

```text
b1fc8113948b518835c2a39ece49553cffe9b30c
v1.17.18
2026-07-09T18:51:29+00:00 release: v1.17.18
```

Release artifact verification:

```bash
curl -fL \
  https://github.com/anomalyco/opencode/releases/download/v1.17.18/opencode-linux-x64.tar.gz \
  -o /tmp/opencode-v1.17.18-linux-x64.tar.gz
sha256sum /tmp/opencode-v1.17.18-linux-x64.tar.gz
mkdir -p /tmp/opencode-v1.17.18-bin
tar -xzf /tmp/opencode-v1.17.18-linux-x64.tar.gz \
  -C /tmp/opencode-v1.17.18-bin
/tmp/opencode-v1.17.18-bin/opencode --version
du -b /tmp/opencode-v1.17.18-linux-x64.tar.gz \
  /tmp/opencode-v1.17.18-bin/opencode
```

Observed:

| Artifact | Result |
| --- | ---: |
| Linux x64 archive | 69,427,073 bytes |
| Extracted binary | 188,979,328 bytes |
| Binary version | `1.17.18` |
| Archive SHA-256 | release API digest matched |

### Inventory And Gates

```bash
cd /tmp/akra-opencode-v1.17.18-source
git ls-files | wc -l
git ls-files '*.ts' '*.tsx' | wc -l
git ls-files -z '*.ts' '*.tsx' \
  | while IFS= read -r -d '' file; do wc -l < "$file"; done \
  | awk '{sum += $1} END {print sum}'
git ls-files | rg '(test|spec)\.(ts|tsx|js|mjs)$' | wc -l

bun install --frozen-lockfile
bun typecheck
mkdir -p /tmp/opencode-bun-bin
ln -sf "$(command -v bun)" /tmp/opencode-bun-bin/bunx
PATH=/tmp/opencode-bun-bin:$PATH GITHUB_ACTIONS=false bun turbo test
PATH=/tmp/opencode-bun-bin:$PATH CI=true GITHUB_ACTIONS=false bun turbo test

cd packages/opencode
bun test --timeout 30000 --only-failures

cd ../app
CI=true bun run test
bun test --preload ./happydom.ts ./src/i18n/parity.test.ts

cd ../desktop
bun test --timeout 30000
```

`/tmp/opencode-bun-bin/bunx` was an external symlink to the installed Bun binary because this host's
Bun installation did not provide a `bunx` executable. The repository was not patched for that
environment limitation.

Observed inventory:

| Inventory | Count |
| --- | ---: |
| Tracked files | 6,230 |
| Tracked TypeScript files | 3,033 |
| Tracked TypeScript LOC | 585,795 |
| JS-family test/spec pathnames | 687 |

Observed gates:

| Gate | Result |
| --- | --- |
| `bun install --frozen-lockfile` | passed; 4,696 packages |
| `bun typecheck` | passed; 30/30 Turbo tasks, 36 packages in scope |
| CI-conditioned monorepo `bun turbo test` | passed; 9/9 Turbo tasks in 6m 7.628s |
| `packages/opencode` | 3,110 pass, 22 skip, 1 todo, 0 fail across 245 files |
| app `test:unit` under `CI=true` | 567 pass, 4 skip, 0 fail across 86 files |
| app `test:browser` under `CI=true` | 27 pass, 0 fail across 10 files |
| desktop direct test | 61 pass, 0 fail across 12 files |
| focused i18n parity without `CI` | 3 pass, 1 fail; Arabic missing three keys |

The initial non-CI monorepo run first failed because `bunx` was absent on the host. With the external
symlink, it reached the Arabic parity failure. That parity suite uses `describe.skipIf(!!process.env.CI)`,
so the CI-conditioned run does not exercise the missing-key case. Expected error logging inside
negative-path tests is not counted as a test failure. The checkout remained clean after both runs.

### Server Authentication

```bash
audit_root="$(mktemp -d /tmp/opencode-auth-audit.XXXXXX)"
pid=''
cleanup() {
  if [[ -n "$pid" ]]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  rm -rf "$audit_root"
}
trap cleanup EXIT
mkdir -p "$audit_root/home" "$audit_root/workspace"
cd "$audit_root/workspace"

HOME="$audit_root/home" \
OPENCODE_DISABLE_PROJECT_CONFIG=1 \
OPENCODE_DISABLE_DEFAULT_PLUGINS=1 \
OPENCODE_DISABLE_MODELS_FETCH=1 \
OPENCODE_SERVER_PASSWORD='AKRA_TEST_PASSWORD_11718' \
  /tmp/opencode-v1.17.18-bin/opencode serve \
  --hostname 127.0.0.1 --port 45123 >"$audit_root/server.log" 2>&1 &
pid=$!
ready=0
for _ in $(seq 1 100); do
  if curl --max-time 1 -fsS -u 'opencode:AKRA_TEST_PASSWORD_11718' \
    http://127.0.0.1:45123/global/health >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 0.1
done
test "$ready" = 1

curl --max-time 10 -sS -o "$audit_root/health-unauth.json" -w '%{http_code}\n' \
  http://127.0.0.1:45123/global/health
curl --max-time 10 -sS -u 'opencode:AKRA_TEST_PASSWORD_11718' \
  -o "$audit_root/health-auth.json" -w '%{http_code}\n' \
  http://127.0.0.1:45123/global/health
cat "$audit_root/health-auth.json"
```

Observed:

```text
401
200
{"healthy":true,"version":"1.17.18"}
```

### Resolved-Secret Canary

The second experiment used fake values created only for this audit. It did not use real provider,
MCP, GitHub, or OpenAI credentials.

```bash
audit_root="$(mktemp -d /tmp/opencode-canary-audit.XXXXXX)"
pid=''
cleanup() {
  if [[ -n "$pid" ]]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  rm -rf "$audit_root"
}
trap cleanup EXIT
mkdir -p "$audit_root/home" "$audit_root/workspace"
cd "$audit_root/workspace"

export AKRA_CANARY_PROVIDER_11718='provider-canary-value-11718'
export AKRA_CANARY_MCP_11718='mcp-canary-value-11718'
export OPENCODE_CONFIG_CONTENT='{
  "provider": {
    "openai": {
      "options": {"apiKey": "{env:AKRA_CANARY_PROVIDER_11718}"}
    }
  },
  "mcp": {
    "akra-canary": {
      "type": "local",
      "command": ["printf", "ready"],
      "enabled": false,
      "environment": {"AKRA_TOKEN": "{env:AKRA_CANARY_MCP_11718}"}
    }
  }
}'
env -u OPENCODE_SERVER_PASSWORD \
  HOME="$audit_root/home" \
  OPENCODE_DISABLE_PROJECT_CONFIG=1 \
  OPENCODE_DISABLE_DEFAULT_PLUGINS=1 \
  OPENCODE_DISABLE_MODELS_FETCH=1 \
  /tmp/opencode-v1.17.18-bin/opencode serve \
  --hostname 127.0.0.1 --port 45124 >"$audit_root/server.log" 2>&1 &
pid=$!
ready=0
for _ in $(seq 1 100); do
  if curl --max-time 1 -fsS \
    http://127.0.0.1:45124/global/health >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 0.1
done
test "$ready" = 1

curl --max-time 20 -sS -o "$audit_root/config.json" -w '%{http_code}\n' \
  http://127.0.0.1:45124/config
curl --max-time 20 -sS -o "$audit_root/provider.json" -w '%{http_code}\n' \
  http://127.0.0.1:45124/provider
curl --max-time 20 -sS -o "$audit_root/v2-provider.json" -w '%{http_code}\n' \
  http://127.0.0.1:45124/api/provider
curl --max-time 20 -sS -o "$audit_root/v2-session.json" -w '%{http_code}\n' \
  'http://127.0.0.1:45124/api/session?limit=1'
curl --max-time 20 -sS -o "$audit_root/legacy-session.json" -w '%{http_code}\n' \
  'http://127.0.0.1:45124/session?limit=1'
rg -o 'provider-canary-value-11718|mcp-canary-value-11718' \
  "$audit_root/config.json" "$audit_root/provider.json"
rg -F 'server is unsecured' "$audit_root/server.log"
```

Observed:

- startup printed `Warning: OPENCODE_SERVER_PASSWORD is not set; server is unsecured.`;
- unauthenticated `/config` returned HTTP `200` and both resolved canary values;
- unauthenticated `/provider` returned HTTP `200` and the provider canary;
- unauthenticated mounted `/api/provider` returned HTTP `200`, but the tested v2 catalog response
  did not contain the configured canary; no dynamic v2 secret-disclosure claim is made;
- unauthenticated mounted `/api/session?limit=1` and legacy `/session?limit=1` both returned HTTP
  `200`, confirming that both route families are present in the release binary;
- the server was bound only to `127.0.0.1` and terminated after capture.

This proves the response boundary for the tested configuration. It does not prove remote exposure
under the default hostname, which is loopback, or exploitation by a model.

### Product Build Footprint

The surface-quality audit ran production application and Electron asset builds from an exact-SHA
export:

```bash
rm -rf /tmp/opencode-v1.17.18-build
mkdir -p /tmp/opencode-v1.17.18-build
git -C /tmp/akra-opencode-v1.17.18-source archive \
  b1fc8113948b518835c2a39ece49553cffe9b30c \
  | tar -x -C /tmp/opencode-v1.17.18-build
cd /tmp/opencode-v1.17.18-build
bun install --frozen-lockfile

/usr/bin/time -f 'elapsed=%e max_rss_kib=%M' \
  bun --cwd packages/app run build 2>&1 | tee /tmp/opencode-app-build.log
find packages/app/dist -type f ! -name '*.map' -printf '%s\n' \
  | awk '{bytes += $1; files += 1} END {print bytes, files}'

/usr/bin/time -f 'elapsed=%e max_rss_kib=%M' \
  bun --cwd packages/desktop run build 2>&1 | tee /tmp/opencode-desktop-build.log
find packages/desktop/out -type f ! -name '*.map' -printf '%s\n' \
  | awk '{bytes += $1; files += 1} END {print bytes, files}'
```

Observed results:

| Build observation | Result |
| --- | ---: |
| Web modules | 2,398 |
| Web build duration | 13.99 s |
| Web entry JavaScript | 2,815.62 kB; 842.34 kB gzip |
| Web CSS | 450.35 kB; about 66 kB gzip |
| Web non-map assets | 42,374,239 bytes, 858 files |
| Electron renderer entry | 6,731.22 kB |
| Bundled Node server chunk | 31,153.76 kB |
| Electron non-map output | 83,169,455 bytes |
| Electron asset build duration | 37.89 s |

The durations are single orientation runs without repeated samples or retained raw timing artifacts.
The sizes are build and bundle facts. None measures packaged startup, steady-state RAM, CPU,
terminal input latency, or long-session behavior.

Desktop installer sizes can be refreshed with:

```bash
curl -fsSL https://api.github.com/repos/anomalyco/opencode/releases/tags/v1.17.18 \
  | jq -r '.assets[]
      | select(.name|test("opencode-desktop.*(dmg|exe|AppImage|deb|rpm)$"))
      | [.name,.size] | @tsv'
```

Primary installer assets ranged from approximately 103.9 MiB to 158.3 MiB. Architecture and
packaging format are confounders; the range is not a cross-product score.

## Immutable Source Ledger

### Product, Release, And Topology

- [README product and install surface](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/README.md#L1-L91)
  identifies the open-source agent, provider breadth, TUI, desktop beta, and distribution. Class:
  `documented` for product positioning, `verified` for checked-in release text.
- [TUI command entry](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/tui.ts#L198-L245)
  selects internal worker or external server mode. Class: `verified`.
- [Default TUI shutdown](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/tui.ts#L217-L295)
  asks its worker server to shut down and terminates it when the client exits. Class: `verified`.
- [TUI worker](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/tui/worker.ts#L23-L78)
  creates the internal server and event bridge. Class: `verified`.
- [Renderer configuration](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/app.tsx#L186-L219)
  targets a 60 Hz renderer, while [SDK context batching](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/context/sdk.tsx#L48-L80)
  coalesces events within a 16 ms window. Class: `verified` design configuration, not measured frame
  performance.
- [Server event route](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/server/routes/instance/httpapi/handlers/event.ts#L12-L84)
  uses an unbounded queue and emits no replayable event ID. The
  [mounted v2 event handler](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/server/src/handlers/event.ts#L9-L50)
  uses capacity 256 but likewise emits no ID and fails the stream on overflow. Class: `verified`.
- [TUI reconnect loop](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/context/sdk.tsx#L82-L116)
  retries external events with backoff but does not perform a full state bootstrap. Class:
  `verified`.
- [Browser reconnect synchronization](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/app/src/context/server-sync.tsx#L372-L395)
  reruns root/directory bootstrap and refreshes session list/status state. It does not force every
  cached session message/part window to reload. Class: `verified`.
- Because external clients lack replay IDs and neither path proves complete cached-transcript
  reconciliation, missing events can leave a selected transcript window stale. Class: `inferred`;
  disconnect loss was not dynamically reproduced.
- [Server route composition](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/server/routes/instance/httpapi/server.ts#L271-L303)
  mounts the v2 server layer beside root, event, instance, and UI routes in the shipped listener.
  Class: `verified`; individual endpoint maturity remains separately labeled.
- [Headless run event loop](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/run.ts#L670-L871)
  derives stdout and idle completion from events and does not use a successful prompt response body
  to reconstruct missing output. Incomplete output or abnormal completion waiting after event loss
  is `inferred`; it was not reproduced.

### TUI, Commands, And Long Sessions

- [Default keybinding registry](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/config/keybind.ts#L41-L117)
  defines leader and session/model/navigation actions. Class: `verified`.
- [Which-key plugin](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/feature-plugins/system/which-key.tsx#L184-L248)
  groups and filters available bindings. Class: `verified`.
- [Session list](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/component/dialog-session-list.tsx#L188-L357)
  implements search, pinning, rename/delete, quick slots, and status. Class: `verified`.
- [Session actions](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/routes/session/index.tsx#L458-L665)
  include compaction, fork, revert, timeline, share, diff, and child navigation. Class: `verified`.
- [Permission UI](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/routes/session/permission.tsx#L20-L87)
  and [question UI](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/routes/session/question.tsx#L35-L94)
  preserve richer structured decisions than Akra's current binary modal. Class: `verified`.
- [Mini-mode lifecycle](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/run/runtime.lifecycle.ts#L305-L400)
  handles split footer, replay, reset, and native scrollback. Class: `verified`.
- [TUI initial hydration](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/context/sync.tsx#L588-L650)
  keeps the latest 100 messages, and the [timeline](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/routes/session/dialog-timeline.tsx#L22-L46)
  reads current memory state. Class: `verified`.
- [Web message paging](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/app/src/context/server-session.ts#L599-L715)
  loads older history in 200-message batches. Class: `verified`.

### Session Continuity, Forks, And Background Work

- [Asynchronous prompt handler](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/server/routes/instance/httpapi/handlers/session.ts#L311-L328)
  forks prompt execution into server scope. Class: `verified`.
- [Attach command](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/attach.ts#L7-L61)
  and [attach startup](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/attach.ts#L107-L146)
  connect to a separately running server and can select a session/fork. Class: `verified` topology;
  an end-to-end live detach/reattach result was not reproduced.
- [Legacy run state](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/session/run-state.ts#L35-L68)
  is an in-memory map and cancels active runs on scope disposal. Class: `verified`.
- [Legacy session fork](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/session/session.ts#L693-L734)
  creates a session and copies messages without setting explicit fork-source lineage. Class:
  `verified`.
- [Background-job registry](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/core/src/background-job.ts#L113-L124)
  explicitly describes process-local, non-durable ownership. Class: `verified` source statement.
- [Task completion injection](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/tool/task.ts#L202-L229)
  adds a synthetic prompt to the parent. Class: `verified`.
- [Legacy runner join behavior](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/effect/runner.ts#L115-L138)
  joins the current run when new work arrives instead of independently scheduling it. A completion
  inserted after the loop's final history read can remain pending. Class: `inferred` from the two
  source paths; the race was not reproduced.
- [v2 runner status](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/core/src/session/runner/llm.ts#L43-L90)
  names incomplete durable ownership, retries, cancellation, plugin parity, and recovery. Class:
  `verified` for mounted code and its stated limitations; maturity: incomplete/experimental.
- [v2 coordinator](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/core/src/session/run-coordinator.ts#L5-L103)
  provides process-local per-session serialization and cross-session concurrency. Class:
  `verified` for the mounted component; durable multi-node behavior remains unimplemented.

### Web, Desktop, IDE, And Sharing

- [Hosted app entry](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/app/src/entry.tsx#L102-L181)
  defaults to a local server. Class: `verified`.
- [Server credential persistence](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/app/src/context/server.tsx#L181-L303)
  uses the application's [local persistence helper](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/app/src/utils/persist.ts#L379-L458)
  for server connection data. Class: `verified`; OS/browser storage protections outside the source
  were not evaluated.
- [Embedded UI serving](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/server/shared/ui.ts#L7-L107)
  serves bundled assets or proxies the hosted app. Class: `verified`.
- [Desktop package](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/desktop/package.json#L12-L75)
  uses Electron 42, while the [README](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/README.md#L67-L83)
  labels desktop beta. Class: `verified` and `documented`.
- [Desktop sidecar](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/desktop/src/main/sidecar.ts#L51-L89)
  uses loopback and generated Basic Auth; the
  [desktop startup path creates the credential](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/desktop/src/main/index.ts#L307-L354).
  Class: `verified`.
- [Desktop window security](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/desktop/src/main/windows.ts#L164-L227)
  and [permission allowlist](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/desktop/src/main/windows.ts#L423-L457)
  set defensive renderer defaults. Class: `verified`.
- [Six-target publish matrix](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/.github/workflows/publish.yml#L220-L400)
  covers macOS, Windows, and Linux x64/arm64. The
  [builder configuration](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/desktop/electron-builder.config.ts#L41-L146)
  configures hardened runtime/notarization and platform packaging. Class: `verified`.
- [VS Code extension](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/sdks/vscode/src/extension.ts#L8-L137)
  launches the CLI and appends file/selection context. Class: `verified`.
- [VS Code test configuration](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/sdks/vscode/.vscode-test.mjs#L1-L5)
  searches generated tests, but the audited extension tree contained no test/spec source file.
  Class: `verified` for configuration and exact-SHA filename inventory.
- [Share upload path](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/share/share-next.ts#L274-L335)
  uploads session, message, part, diff, and model data to the share service. Class: `verified`.
  Hosted access control, retention, and server-side redaction were not verified.
- [GitHub Action share input](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/github/action.yml#L16-L18)
  states that public repositories default to sharing, and the
  [handler enables it when `share` is unset and the repository is public](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/github.handler.ts#L511-L515).
  Class: `verified`; hosted access/retention policy remains `unverified`.
- [Nix desktop runtime](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/nix/desktop.nix#L1-L16)
  uses Electron 41 while the package manifest uses Electron 42.3.3. The
  [Nix evaluation job](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/.github/workflows/nix-eval.yml#L40-L77)
  treats desktop evaluation as optional/warning. Class: `verified` version drift; runtime impact is
  `unverified`.

### Worktrees, GitHub, And Delivery

- [Worktree creation](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/worktree/index.ts#L174-L293)
  creates `opencode/<slug>` workspaces and can run startup commands. Class: `verified`.
- [Worktree reset](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/worktree/index.ts#L525-L610)
  hard-resets and cleans non-primary worktrees and submodules. Class: `verified`.
- [GitHub event selection](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/github.handler.ts#L376-L427)
  covers issues, pull requests, review comments, schedules, and dispatch. Class: `verified`.
- [GitHub delivery](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/github.handler.ts#L519-L630)
  stages, commits, pushes, and creates/updates a PR. Class: `verified`.
- [Generated workflow](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/github.handler.ts#L329-L367)
  uses `anomalyco/opencode/github@latest`. Class: `verified` mutable dependency.
- [Git config credential injection and restore](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/github.handler.ts#L1013-L1038)
  writes an extra header and returns early when no previous header existed. Class: `verified`.
- [GitHub execution ordering](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/github.handler.ts#L475-L531)
  performs token/configuration setup and attachment work before the actor authorization decision.
  Class: `verified`.
- Agent or prompt exfiltration of that active token is possible from the inspected capabilities.
  Class: `inferred`; no real token was exposed or exfiltrated by this audit.
- No reviewed-check/integration/cleanup authority comparable to Akra was found in the inspected
  handler. Class: `inferred` from the pinned implementation and repository search, not a claim about
  external GitHub policy.

### Authentication, Secrets, Permissions, And Extensions

- [Server auth configuration](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/server/auth.ts#L17-L47)
  provides optional password-variable-driven Basic Auth. Class: `verified`.
- [Authorization middleware](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/server/routes/instance/httpapi/middleware/authorization.ts#L101-L149)
  passes requests when authentication is not required. Class: `verified`.
- [Credential extraction](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/server/routes/instance/httpapi/middleware/authorization.ts#L73-L82)
  accepts either Basic Auth or an `auth_token` query parameter. Class: `verified`; URL history/proxy
  log exposure is `inferred`.
- [Serve command](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/serve.ts#L13-L23)
  warns but starts without a password. Class: `verified` and dynamically reproduced.
- [Network options](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/network.ts#L7-L20)
  default to loopback; [mDNS/config resolution](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/network.ts#L62-L79)
  can select `0.0.0.0`. Class: `verified`.
- [Config handler](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/server/routes/instance/httpapi/handlers/config.ts#L9-L29)
  returns the effective config, while [variable substitution](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/config/variable.ts#L33-L90)
  resolves environment and file references. Class: `verified` and dynamically reproduced.
- [Provider config schema](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/core/src/v1/config/provider.ts#L76-L120)
  includes `options.apiKey`; [MCP config](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/core/src/v1/config/mcp.ts#L6-L59)
  includes local environment, remote headers, and OAuth client secret. Class: `verified`.
- [Provider public mapping](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/provider/provider.ts#L1046-L1085)
  retains the legacy `key`, and the [provider handler](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/server/routes/instance/httpapi/handlers/provider.ts#L40-L58)
  returns that mapping. Class: `verified` and dynamically reproduced for an environment canary.
- [Mounted v2 provider endpoints](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/protocol/src/groups/provider.ts#L7-L31)
  return the [catalog provider object](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/server/src/handlers/provider.ts#L8-L29), whose
  [schema retains request headers and body](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/schema/src/provider.ts#L46-L61).
  [v1 provider lowering](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/core/src/v1/config/provider-options.ts#L29-L114)
  can place API keys or auth tokens in `Authorization`, `x-api-key`, and `api-key` headers. Class:
  `verified` source path; configured-secret disclosure through this v2 response is `unverified`.
- [Default agent permissions](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/agent/agent.ts#L119-L136)
  set wildcard allow with `.env` read asks. Class: `verified`.
- [Shell file parsing and checks](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/tool/shell.ts#L263-L290)
  and [permission request path](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/tool/shell.ts#L378-L425)
  do not apply the direct-read permission to an in-workspace `cat .env`. Class: `verified` source
  composition; prompt bypass impact is `inferred`.
- [Plugin input](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/plugin/index.ts#L139-L177)
  exposes client, project/worktree, and shell capability. Class: `verified` trusted-code boundary.
- [Server plugin API](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/plugin/src/index.ts#L222-L334)
  exposes provider/auth, chat parameters/headers, tools, permissions, shell environment, tool
  execution, and compaction hooks, while the
  [TUI plugin API](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/plugin/src/tui.ts#L53-L120)
  extends terminal behavior. Class: `verified` capability breadth and trusted-code surface.
- [Local MCP process](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/mcp/index.ts#L340-L369)
  inherits `process.env`. Class: `verified`.
- [Legacy provider auth storage](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/auth/index.ts#L58-L89)
  and [MCP OAuth storage](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/mcp/auth.ts#L59-L101)
  use mode-`0600` plaintext JSON. The [v2 credential table](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/core/src/credential/sql.ts#L5-L13)
  stores credential values as JSON in SQLite. Class: `verified`; filesystem/database encryption
  supplied outside these paths is `unverified`.

### Performance And Quality

- [Performance suite README](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/app/e2e/performance/README.md#L1-L55)
  defines manual renderer metrics, machine dependence, and packaged-Electron limits. Class:
  `documented`.
- [Performance Playwright config](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/app/e2e/performance/playwright.config.ts#L1-L20)
  uses a production build and one worker. Class: `verified`.
- [Unit and generated-client CI](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/.github/workflows/test.yml#L23-L80)
  runs Linux and Windows tests plus Linux-only generated API checks. Class: `verified`.
- [E2E CI](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/.github/workflows/test.yml#L82-L151)
  runs Chromium on Linux and Windows with retry artifacts. Class: `verified`.
- [Arabic parity suite](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/app/src/i18n/parity.test.ts#L1-L85)
  skips under `CI`; the three-key mismatch was reproduced locally. Class: `verified`.
- [Desktop package scripts](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/desktop/package.json#L12-L24)
  omit `test`, although direct tests passed. Class: `verified`.
- [Publish dependencies](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/.github/workflows/publish.yml#L408-L521)
  do not depend structurally on test/typecheck jobs. Class: `verified`; branch protection and
  external release policy are `unverified`.

## Akra Baseline Evidence

- [Direct app-server connection](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/src/adapter/outbound/app_server/connection.rs#L968-L1066)
  initializes the official stdio server. Class: `verified`.
- [Filtered app-server environment](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/src/adapter/outbound/app_server/connection.rs#L443-L520)
  and [spawn environment policy](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/src/adapter/outbound/app_server/connection.rs#L725-L773)
  clear and allowlist the child environment with exact API-key opt-in by default. The
  [configuration resolver](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/src/adapter/outbound/app_server/connection.rs#L377-L405)
  accepts exact `AKRA_APP_SERVER_PROCESS_ENVIRONMENT=all` as an explicit full-environment override.
  Class: `verified`; default and override must not share one safety claim.
- [Approval domain](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/src/domain/conversation.rs#L233-L269)
  preserves bounded request facts but exposes only accept/decline. Class: `verified`.
- [Fail-closed server requests](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/src/adapter/outbound/app_server/connection.rs#L1817-L1884)
  decline uninspectable file-change approval, user input, and MCP elicitation. Class: `verified`.
- [Admin server bootstrap](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/src/adapter/inbound/admin_api/mod.rs#L75-L125)
  binds loopback and requires an explicit token when noninteractive. Class: `verified`.
- [Admin security](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/src/adapter/inbound/admin_api/security.rs#L17-L115)
  uses a 64-character capability, random `.localhost` origin, digest comparison, and bounded
  session lifetime. Class: `verified`.
- [TUI command registry](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/src/adapter/inbound/tui/app/inline_shell_commands.rs#L38-L92)
  is already the source for typed and palette-accepted commands. Class: `verified`.
- [Official app-server schema fork](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/schema/codex_app_server_protocol.v2.schemas.json#L1320-L1360)
  and [fork lineage field](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/schema/codex_app_server_protocol.v2.schemas.json#L16630-L16655)
  exist, while no Akra request/port path was found. Class: `verified` for schema, `inferred` for the
  repository-wide implementation absence.
- [Official thread projection](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/schema/codex_app_server_protocol.v2.schemas.json#L16734-L16749)
  can populate turns for read/resume/fork, but the
  [schema explicitly describes stored ThreadItems as lossy](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/schema/codex_app_server_protocol.v2.schemas.json#L19581-L19597)
  because not all agent interactions, including command execution, are persisted. Class: `verified`;
  recovery must distinguish protocol-guaranteed fields from event-only history.
- [Current parallel delivery contract](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/docs/supersession/current-contract.md#L100-L164)
  records frozen source/target identity, review/check default gates, serialized integration, remote
  verification, and cleanup. Class: `verified` contract backed by the referenced implementation.
- [Existing jcode slices](../jcode/gap-matrix.md#implementation-slices) own performance, live
  execution, steering/restart, validation, delivery outcomes, Admin metrics/controls, parallel
  activity, review response, and session provenance. Class: `documented` Akra plan.
- [Existing Canvas slices](../agent-canvas/gap-matrix.md#implementation-slices) own current activity,
  truthful diorama, automation intake/run provenance, and read-only remote nodes. Class:
  `documented` Akra plan.

## Audit Limits

- No authenticated provider/model inference was run, so model quality, tool success, token latency,
  and permission behavior under a real LLM are unverified.
- No fair interactive Akra/OpenCode benchmark was run. Startup, input, first token, steady-state
  memory, CPU, renderer latency, and long-session stability have no winner.
- The secret canary used fake values and loopback. No real credential was exposed.
- Dynamic secret reproduction covered only config-supplied `openai.options.apiKey`, a disabled local
  MCP `environment` value, legacy `/config`, and legacy `/provider`. `{file:...}`, auth-file keys,
  remote MCP headers/OAuth, actual MCP spawn, v2 configured secrets, and GitHub tokens were not
  dynamically tested.
- No built-in TLS termination was evaluated. A non-loopback HTTP deployment needs a separately
  operated confidentiality boundary; its correctness is unverified.
- `.env` shell bypass, GitHub token exfiltration, SSE overflow/loss, background-injection races,
  fork/revert concurrency, malicious plugins/MCP/LSP, and Windows process-tree cancellation were not
  dynamically exploited.
- Hosted share access control, retention, deletion, redaction, and enterprise policy were not
  evaluated.
- GitHub branch protection, private organization policy, marketplace scanning, and external release
  approval may add gates not visible in the repository.
- Code presence in the experimental v2 packages does not prove that the released default product
  uses it.
- A clean direct package test result does not prove every package or packaged desktop path is
  covered by the release workflow.
