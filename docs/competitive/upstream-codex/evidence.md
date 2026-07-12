# Upstream OpenAI Codex v0.144.1 Evidence Ledger

This ledger separates immutable source inspection, released-binary reproduction, official
documentation, inference, and unverified behavior. Product conclusions are in
[analysis.md](analysis.md); Akra decisions and implementation contracts are in
[gap-matrix.md](gap-matrix.md).

## Snapshot

| Field | Value |
| --- | --- |
| Product | OpenAI Codex |
| Official repository | <https://github.com/openai/codex> |
| Release | [`rust-v0.144.1`](https://github.com/openai/codex/releases/tag/rust-v0.144.1) |
| Annotated tag object | `db75c19352d29ef29c17dbcf73a7244f1b1a8d10` (peeled below) |
| Peeled release commit | `44918ea10c0f99151c6710411b4322c2f5c96bea` |
| Source commit time | 2026-07-09 15:10:27 -07:00 |
| Release published | 2026-07-09 23:02:40 UTC |
| Previous release commit | `767822446c7a594caa19609ca435281a9ec67e0d` (`rust-v0.144.0`) |
| Audit date | 2026-07-12 (Asia/Seoul) |
| Akra baseline | `226e4794b84107704378ecc1ea65f7d5c27750e5` on `prerelease` |
| Akra version | 1.3.5 |
| Environment | Ubuntu 24.04.2 WSL2, Linux 6.18.33.2, x86_64 |
| Toolchain | Rust/Cargo 1.95.0, Node 24.14.1, npm 11.6.1 |

The source began as a clean detached checkout. Running Cargo against the release workspace rewrote
workspace package version entries in the temporary `Cargo.lock` from `0.0.0` to `0.144.1`.
Immutable links and source conclusions therefore use the peeled commit object, and no claim depends
on the later temporary working-tree bytes. The original committed lockfile SHA-256 was
`175793a40a3147db1fee08fd9db0acc59312c344b3513dd7ee316f5446d8119e`.

## Evidence Classes

This audit uses the parent [evidence rules](../README.md#evidence-classes):

- `verified/source`: read at the exact peeled commit;
- `verified/artifact`: reproduced with a release asset whose checksum matched the release API;
- `verified/local`: observed in this environment but not preserved as a portable release artifact;
- `documented`: stated by official Codex documentation without a matching local exercise;
- `proposed`: explicitly experimental, under development, beta, or unsupported;
- `inferred`: a bounded conclusion from inspected control flow or several verified facts;
- `unverified`: not exercised or not sufficiently identified to support a conclusion.

Source presence proves implementation, not feature stage, default enablement, operational scale, or
end-user performance. A generated experimental schema proves vocabulary, not successful runtime
negotiation. A first-party TUI path proves topology, not a latency or memory winner.

## Release Identity And Artifacts

### Source checkout

```bash
git clone https://github.com/openai/codex.git \
  /tmp/akra-upstream-codex-v0.144.1-source
git -C /tmp/akra-upstream-codex-v0.144.1-source \
  checkout --detach rust-v0.144.1
git -C /tmp/akra-upstream-codex-v0.144.1-source rev-parse HEAD
git -C /tmp/akra-upstream-codex-v0.144.1-source \
  show -s --format='%H%n%aI%n%cI%n%s' HEAD
```

Observed:

```text
44918ea10c0f99151c6710411b4322c2f5c96bea
2026-07-09T15:10:27-07:00
2026-07-09T15:10:27-07:00
## Bug Fixes
```

The [workspace version](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/Cargo.toml#L132-L139)
is `0.144.1`. Class: `verified/source`.

### Patch scope

```bash
git -C /tmp/akra-upstream-codex-v0.144.1-source \
  diff --shortstat 767822446c7a594caa19609ca435281a9ec67e0d..44918ea10c0f99151c6710411b4322c2f5c96bea
git -C /tmp/akra-upstream-codex-v0.144.1-source \
  diff --name-status 767822446c7a594caa19609ca435281a9ec67e0d..44918ea10c0f99151c6710411b4322c2f5c96bea
```

Observed: eight files changed, 549 insertions, 161 deletions. Paths were the workspace manifest,
code-mode remote-session implementation/tests, core code-mode implementation/tests, and installer
implementation/tests. This verifies that v0.144.1 is a narrow patch. It does not date every audited
app-server or TUI feature to that patch. Class: `verified/source`.

### Released Linux assets

The release API reported these exact assets. The two downloaded archives matched the API digest:

| Asset | Archive bytes | SHA-256 | Extracted bytes | Extracted SHA-256 |
| --- | ---: | --- | ---: | --- |
| `codex-x86_64-unknown-linux-musl.tar.gz` | 109,308,813 | `84091ae20c65fcc7d4120db97d1bd57d7ff8df9c7609fb781c78c2ebbd4f5a28` | 298,520,624 | `a96f944d1a596dbfb7fdd84f482be5c50e34b04bb371126840d873e4ebf26902` |
| `codex-app-server-x86_64-unknown-linux-musl.tar.gz` | 93,163,891 | `a6705726bb5ca1c9e803231dcebb6dae1aaa7be13b83270f4762a30853c13e58` | 248,062,016 | `b68de8340b2ccb8aeb23b6f33a4b4dff4f203ff84f6d82b6feedf334aba9a4fb` |

Both extracted binaries reported `0.144.1`. They were stripped x86_64 static PIE executables with
non-executable stack, GNU RELRO, and immediate binding. The downloaded sample did not include its
separate `.sigstore` asset, so the audit verified release digest identity but did not independently
run cosign verification. Class: `verified/artifact`.

Representative reproduction:

```bash
ARTIFACT_DIR=/tmp/akra-upstream-codex-v0.144.1-bin
mkdir -p "$ARTIFACT_DIR"
curl -fL \
  https://github.com/openai/codex/releases/download/rust-v0.144.1/codex-x86_64-unknown-linux-musl.tar.gz \
  -o "$ARTIFACT_DIR/codex.tar.gz"
curl -fL \
  https://github.com/openai/codex/releases/download/rust-v0.144.1/codex-app-server-x86_64-unknown-linux-musl.tar.gz \
  -o "$ARTIFACT_DIR/codex-app-server.tar.gz"
printf '%s  %s\n' \
  84091ae20c65fcc7d4120db97d1bd57d7ff8df9c7609fb781c78c2ebbd4f5a28 \
  "$ARTIFACT_DIR/codex.tar.gz" | sha256sum --check --strict -
printf '%s  %s\n' \
  a6705726bb5ca1c9e803231dcebb6dae1aaa7be13b83270f4762a30853c13e58 \
  "$ARTIFACT_DIR/codex-app-server.tar.gz" | sha256sum --check --strict -
tar -tzf "$ARTIFACT_DIR/codex.tar.gz"
tar -tzf "$ARTIFACT_DIR/codex-app-server.tar.gz"
tar -xzf "$ARTIFACT_DIR/codex.tar.gz" --directory "$ARTIFACT_DIR"
tar -xzf "$ARTIFACT_DIR/codex-app-server.tar.gz" --directory "$ARTIFACT_DIR"
CLI_BIN="$ARTIFACT_DIR/codex-x86_64-unknown-linux-musl"
APP_SERVER_BIN="$ARTIFACT_DIR/codex-app-server-x86_64-unknown-linux-musl"
"$CLI_BIN" --version
"$APP_SERVER_BIN" --version
```

Each of these two simple tar archives contains one binary. Separate complete package archives are
also published and include the `codex-resources` layout. The distinction is material on Linux.

### Linux single-binary sandbox probe

The README presents the simple platform tar as a direct installation path. The
[Linux launcher](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/linux-sandbox/src/launcher.rs#L36-L65)
requires either system `bwrap` or an adjacent bundled helper. The simple tar contains neither.
The complete [package layout](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/scripts/codex_package/layout.py#L34-L94)
includes the helper and [validates it](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/scripts/codex_package/layout.py#L136-L163).

On the audit host, which had no system `bwrap`, the released simple CLI archive failed this command
with exit 101:

```bash
mkdir -p /tmp/codex-home
CODEX_HOME=/tmp/codex-home "$CLI_BIN" \
  sandbox -C /tmp -P :read-only /usr/bin/true
```

The simple app-server artifact's stable `command/exec` path returned `exitCode: 101` for the same
missing-helper condition. This is a shipped artifact/package-shape defect on hosts without system
Bubblewrap. It does not apply to the complete package archive or hosts with a compatible system
helper. Class: `verified/artifact`.

## Binary Surface Reproduction

### CLI and subcommands

```bash
BIN=/tmp/akra-upstream-codex-v0.144.1-bin/codex-x86_64-unknown-linux-musl
APP_SERVER=/tmp/akra-upstream-codex-v0.144.1-bin/codex-app-server-x86_64-unknown-linux-musl

"$BIN" --version
"$APP_SERVER" --version
"$BIN" --help
"$BIN" app-server --help
"$BIN" app-server daemon --help
"$BIN" remote-control --help
"$BIN" doctor --help
"$BIN" resume --help
"$BIN" fork --help
```

Observed CLI families included interactive TUI, `exec`, `review`, login/logout, MCP, plugins,
MCP-server, app-server, remote-control, doctor, sandbox, resume/archive/delete/unarchive/fork,
cloud, and features. App-server and remote-control were labeled experimental. App-server exposed
stdio, Unix socket, WebSocket, and off modes plus capability-token and signed-bearer WebSocket auth.
Class: `verified/artifact`.

### Feature catalog

```bash
CODEX_HOME="$(mktemp -d)" "$BIN" features list
```

The output contained 92 feature rows. Source inspection distinguished stable, experimental,
under-development, and removed compatibility entries. Examples at this snapshot:

- stable/default-on included first-generation multi-agent, hooks, goals, apps/plugins, remote
  compaction v2, Linux unified exec, and shell snapshot/tool behavior;
- experimental/default-off included memories and the network proxy;
- under-development/default-off included multi-agent V2, fanout, permission-request tools,
  exec-permission approvals, local thread compression, and token/rollout budgets;
- removed entries such as old steer/collaboration switches can remain visible as compatibility
  no-ops.

The [feature-stage contract](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/features/src/lib.rs#L34-L50)
supports this distinction. Class: `verified/artifact` for the rows and `verified/source` for stage
meaning.

## Schema And Handshake Evidence

### Stable versus experimental schema

```bash
CODEX_HOME="$(mktemp -d)" "$BIN" app-server generate-json-schema \
  --out /tmp/codex-schema-stable
CODEX_HOME="$(mktemp -d)" "$BIN" app-server generate-json-schema \
  --out /tmp/codex-schema-experimental --experimental

jq '.oneOf | length' /tmp/codex-schema-stable/ClientRequest.json
jq '.oneOf | length' /tmp/codex-schema-stable/ServerNotification.json
jq '.oneOf | length' /tmp/codex-schema-stable/ServerRequest.json
jq '.oneOf | length' /tmp/codex-schema-stable/ClientNotification.json
jq '.oneOf | length' /tmp/codex-schema-experimental/ClientRequest.json
jq '.oneOf | length' /tmp/codex-schema-experimental/ServerNotification.json
jq '.oneOf | length' /tmp/codex-schema-experimental/ServerRequest.json
jq '.oneOf | length' /tmp/codex-schema-experimental/ClientNotification.json
```

Observed:

| Schema | Client request | Server notification | Server request | Client notification |
| --- | ---: | ---: | ---: | ---: |
| stable | 87 | 68 | 10 | 1 |
| `--experimental` | 122 | 68 | 11 | 1 |

The app-server [gating contract](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server/README.md#L2118-L2177)
requires initialize capability for experimental methods/fields even if a client has their schema.
Class: `verified/artifact` and `verified/source`.

### Akra schema normalization

Akra's checked-in schema metadata identifies `codex-cli 0.144.0` and was generated with experimental
definitions. A v0.144.1 experimental export was passed through
`scripts/normalize_codex_app_server_schema.mjs`. After controlling source CLI and generated-date
metadata, canonical `jq -S '.definitions'` output was byte-identical with SHA-256
`99ade25a6cbfa75a35abc5841cb6d6f98b90d1a1e9b2ec3eca2772a3bde968ac`. Class:
`verified/local`.

This result permits only this conclusion: v0.144.1 did not change the normalized experimental
definition vocabulary from Akra's v0.144.0 snapshot. It does not prove stable-vs-experimental
contract accuracy, runtime capability enablement, payload reduction, or source-to-sink fidelity.

### Positive isolated handshake

The checked-in [probe harness](scripts/probe-app-server.mjs) launched the released app-server under
a fresh `CODEX_HOME`/`HOME` and a child environment limited to locale, path, certificate, and time
zone variables. It sent `initialize` with experimental capability enabled, then `initialized`. The
following reads returned successful JSON-RPC results without model authentication:

| Method | Observation |
| --- | --- |
| `account/read` | empty/no active account state returned |
| `thread/list` | empty catalog returned |
| `thread/loaded/list` | empty loaded list returned |
| `model/list` | seven model rows returned with explicit limit 100 |
| `collaborationMode/list` | two modes returned |
| `app/list` | successful list response |
| `skills/list` | successful list response |
| `plugin/list` | successful list response |
| `remoteControl/status/read` | disabled status returned |
| `permissionProfile/list` | three profiles returned |
| `hooks/list` | successful list response |
| `config/read` | effective isolated config returned |
| `mcpServerStatus/list` | successful list response |
| `experimentalFeature/list` | 92 feature rows returned |

Initialize returned the expected user agent, isolated Codex home, and platform. These observations
verify request availability in this binary and state. They do not verify authenticated fields,
network-backed services, live model execution, or cross-platform behavior. Class: `verified/local`.

`permissionProfile/list` itself is stable, but the source marks named
[`thread/start.permissions`](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L86-L94)
selection and response
[`activePermissionProfile`](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L185-L196)
provenance as experimental. Stable `thread/start` also retains a generic
[`config` map](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L87-L96),
whose entries are merged as
[CLI overrides](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server/src/config_manager.rs#L220-L249).
Consequently, an existing custom default or `config.default_permissions` can select a custom profile
on a stable connection, but only legacy sandbox is returned and typed profile identity/provenance is
absent. This generic escape hatch is not equivalent to a dedicated stable named-profile contract.
Class: `verified/source`.

### Negative handshake and capability probes

Separate fresh processes produced:

- request before initialize: JSON-RPC `-32600`, `Not initialized`;
- experimental request without capability: JSON-RPC `-32600` naming the required
  `experimentalApi` capability;
- repeated initialize: JSON-RPC `-32600`, `Already initialized`;
- the control connection received one initial `remoteControl/status/changed`; an otherwise matching
  connection that opted out received zero.

With `experimentalApi: false`, stable `fs/readFile` read `/etc/hostname`, while `process/spawn`
returned `-32600` because that method is experimental. The read used a disposable direct request;
no write/remove/process-spawn mutation was exercised. Reproduce these positive and negative paths
with the following command. It starts multiple fresh default app-server processes; enabled plugin
startup tasks can repeat public GitHub marketplace reads into disposable homes, so this is not an
offline probe.

```bash
APP_SERVER_BIN=/tmp/akra-upstream-codex-v0.144.1-bin/codex-app-server-x86_64-unknown-linux-musl
printf '%s  %s\n' \
  b68de8340b2ccb8aeb23b6f33a4b4dff4f203ff84f6d82b6feedf334aba9a4fb \
  "$APP_SERVER_BIN" | sha256sum --check --strict -
CODEX_APP_SERVER="$APP_SERVER_BIN" \
  node docs/competitive/upstream-codex/scripts/probe-app-server.mjs
```

The script emits only bounded summaries, counts, error codes/messages, and the artifact digest; it
does not print config, credentials, file contents, or raw environment. Class: `verified/local`.

## Immutable Source Ledger

### Runtime and first-party client topology

- [App-server client transport model](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-client/README.md#L27-L43)
  defines typed in-process request/event channels while retaining JSON-RPC response envelopes.
  Class: `verified/source`.
- [Backpressure and shutdown](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-client/README.md#L58-L67)
  defines bounded queues, overload, lag, and bounded shutdown. The targeted client tests below
  exercised representative cases. Class: `verified/source`.
- [TUI target types](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/tui/src/lib.rs#L260-L279)
  define embedded, local-daemon, and explicit remote paths. The
  [launch selector](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/tui/src/lib.rs#L799-L920)
  probes a compatible default socket only when launch config is replayable and otherwise falls back
  to embedded. Class: `verified/source`.
- [Remote disconnect handling](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-client/src/remote.rs#L396-L456)
  converts invalid frames, disconnect, transport failure, and closure into events. No general
  reconnect loop was found in this client. Reconnect absence is an inspected-inventory conclusion,
  not proof no outer surface can reconnect. Class: `verified/source` plus bounded `inferred`.
- [Daemon status](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-daemon/README.md#L1-L15)
  marks the daemon experimental and Unix-only, while its
  [update lifecycle](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-daemon/README.md#L34-L70)
  is not reboot-persistent and may restart app-server. Class: `proposed` product surface with
  `verified/source` implementation.

### App-server protocol

- [Thread/turn model](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/v2/thread_data.rs#L167-L275)
  carries session and fork lineage, history/source/agent/Git provenance, turn items/view, status,
  error, timestamps, and duration. Class: `verified/source`.
- [Turn statuses](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/v2/turn.rs#L30-L38)
  are `completed`, `interrupted`, `failed`, and `inProgress`. Class: `verified/source`.
- [Error notification](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/v2/notification.rs#L38-L48)
  nests a typed error and states that `willRetry: true` does not interrupt the turn. Class:
  `verified/source`.
- [Thread item vocabulary](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/v2/item.rs#L222-L396)
  defines 18 typed item variants at the snapshot. Class: `verified/source`.
- [Model capability response](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/v2/model.rs#L40-L146)
  exposes defaults, reasoning options, input modalities, and service-tier/capability facts. Class:
  `verified/source`.
- [Reasoning effort vocabulary](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/protocol/src/openai_models.rs#L40-L50)
  includes `max` and `ultra` at this snapshot. Class: `verified/source`.
- [Applied thread response](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L167-L201)
  returns effective model/provider/cwd/approval/sandbox/reasoning/service-tier state. Class:
  `verified/source`.
- [Approval lifecycle](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server/README.md#L1444-L1498)
  shows `serverRequest/resolved` after response or lifecycle cleanup. It proves the server-side
  pending request cleared, not that a chosen decision was accepted or applied. Class:
  `verified/source` for the sequence and `inferred` for the narrow evidence interpretation.

### Sessions, context, and agents

- [Rollout reconstruction](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/session/rollout_reconstruction.rs#L113-L190)
  reconstructs replacement history, rollback, world state, settings, and compaction state. Class:
  `verified/source`.
- [Fork boundary](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/thread_manager.rs#L128-L156)
  describes mid-turn snapshot limits and inserts an aborted-turn boundary. Class:
  `verified/source`.
- [Compaction replacement](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/compact.rs#L322-L376)
  and [recent-input cap](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/compact.rs#L585-L658)
  implement local replacement and a 20k-token recent-user-message bound. Class: `verified/source`.
- [Pre-turn compaction TODO](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/session/turn.rs#L142-L165)
  omits the new input/context diff from the initial estimate; later
  [mid-turn logic](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/session/turn.rs#L300-L370)
  can react after sampling begins. A large-input first-attempt failure is plausible but was not
  reproduced. Class: `verified/source` plus `inferred` risk.
- [V1 subagent inheritance](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/tools/handlers/multi_agents_common.rs#L154-L231)
  preserves runtime policy and environment facts for children. Class: `verified/source`.
- [V2 stage](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/features/src/lib.rs#L1035-L1046),
  [depth gate difference](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/tools/spec_plan.rs#L339-L351),
  and [execution limiter](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/agent/control/execution.rs#L59-L101)
  support the bounded source concerns in the analysis. V2 is under development and the suspected
  concurrency overshoot was not load-reproduced. Class: `proposed`, `verified/source`, and
  explicitly limited `inferred`.

### TUI approval behavior

- [Approval context and choices](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/tui/src/bottom_pane/approval_overlay.rs#L621-L940)
  show thread/environment context and one-shot/session/persistent/decline/abort choices when
  supplied. Class: `verified/source`.
- [Protocol decision meaning](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/protocol/src/protocol.rs#L4022-L4057)
  distinguishes decline-and-continue from abort, while the
  [normal default list](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/protocol/src/approvals.rs#L277-L331)
  can omit decline. Class: `verified/source`.
- [Pending approval storage](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/tui/src/bottom_pane/approval_overlay.rs#L158-L214)
  uses `push`, and [advance](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/tui/src/bottom_pane/approval_overlay.rs#L474-L525)
  uses `pop`. Initial batches reverse insertion elsewhere, but arrivals during a visible modal can
  favor newer requests. Class: `verified/source` plus bounded `inferred` starvation risk.

### Security and remote boundaries

- [Transport contract](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server/README.md#L20-L53)
  labels WebSocket experimental/unsupported and documents stdio/Unix/WS shapes. Class:
  `documented` with source support.
- [Unix socket creation](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-transport/src/transport/unix_socket.rs#L21-L43)
  sets mode `0600`. Class: `verified/source`.
- [WebSocket protections](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-transport/src/transport/websocket.rs#L89-L152)
  reject unauthenticated non-loopback configuration and Origin-bearing upgrades. The released
  binary also rejected `--listen ws://0.0.0.0:0` without auth with exit 1. Class:
  `verified/source` and `verified/artifact`.
- [Remote client token policy](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-client/src/remote.rs#L117-L124)
  does not put a token on non-loopback plain `ws`. Class: `verified/source`.
- [Stable filesystem methods](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/common.rs#L743-L789)
  and their [unsandboxed processor](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server/src/request_processors/fs_processor.rs#L64-L191)
  operate on host paths. `thread/shellCommand` is likewise documented as full-access/unsandboxed.
  Class: `verified/source`.
- [Process spawn](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/common.rs#L1056-L1087)
  is experimental; its [processor](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server/src/request_processors/process_exec_processor.rs#L68-L140)
  is unsandboxed and inherits app-server environment. Class: `verified/source`.
- [Shell environment defaults](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/protocol/src/config_types.rs#L187-L243)
  select `inherit: All` and disable default exclusions, while
  [filter rules](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/protocol/src/shell_environment.rs#L46-L109)
  show that key/secret/token patterns are therefore not applied by default. Class:
  `verified/source`.
- [Trust-dependent sandbox default](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/config/permissions.rs#L48-L58)
  selects the workspace profile after either explicit `Trusted` or `Untrusted` state and read-only
  only when the trust level is absent. Both profiles retain root read;
  [workspace-write](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/protocol/src/permissions.rs#L568-L603)
  adds bounded writes. Class: `verified/source`.
- [Auth storage default](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/config/src/types.rs#L87-L100)
  is plaintext file storage. The [payload](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/login/src/auth/storage.rs#L38-L61)
  can contain API/OAuth/PAT/private-key material. On Unix the create path
  [requests mode `0600`](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/login/src/auth/storage.rs#L202-L218),
  but that create mode does not repair an existing file with looser permissions. Keyring and
  encrypted local secret options exist. Class: `verified/source`.
- [Rollout policy](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/rollout/src/policy.rs#L36-L57)
  retains sensitive interaction content, while the
  [recorder create path](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/rollout/src/recorder.rs#L1530-L1543)
  does not itself force `0600`. Exposure depends on parent permissions and umask and was not
  reproduced. Class: `verified/source` plus conditional `inferred` risk.
- [Remote-control persistence](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/state/migrations/0024_remote_control_enrollments.sql#L1-L10)
  stores server/environment identifiers, not the remote token. Class: `verified/source`.

### Memory safety boundaries

- [Memory feature stage](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/features/src/lib.rs#L925-L934)
  is experimental/default-off. Class: `proposed`.
- [Phase-one filtering](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/memories/write/src/phase1.rs#L403-L473)
  and [secret sanitizer patterns](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/secrets/src/sanitizer.rs#L4-L21)
  are best-effort, not general secret detection. Class: `verified/source`.
- [Rate-limit guard](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/memories/write/src/guard.rs#L9-L46)
  permits processing when the check fails. Class: `verified/source`.
- [Phase-two worker restrictions](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/memories/write/src/phase2.rs#L297-L347)
  disable network and extensions and restrict writes. Class: `verified/source`.

### Backpressure and lifecycle

- [Core channel construction](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/session/mod.rs#L468-L539)
  bounds submission but creates an unbounded event channel; the
  [forwarder](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/session/mod.rs#L1946-L1983)
  precedes the [bounded app-server channel](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server/src/lib.rs#L447-L467).
  Memory accumulation under a stalled client is a source inference and was not stress-reproduced.
  Class: `verified/source` plus `inferred`.
- [Rollout queue and flush](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/rollout/src/recorder.rs#L832-L930)
  and [write retry](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/rollout/src/recorder.rs#L1545-L1654)
  provide meaningful durability. Class: `verified/source`.
- Side-thread cleanup interrupts and unsubscribes immediately, while
  [thread lifecycle unload](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server/src/request_processors/thread_lifecycle.rs#L4-L105)
  uses a per-thread 30-minute no-subscriber-and-inactive delay. Temporary accumulation before that
  deadline is a bounded inference; a lifetime leak claim is rejected. Class: `verified/source` plus
  `inferred`.

### Release and quality

- [Release toolchain setup](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/.github/workflows/rust-release.yml#L165-L174)
  pins Rust 1.95. The
  [Linux/build path](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/.github/workflows/rust-release.yml#L216-L362)
  builds and hashes Bubblewrap, builds Codex binaries, archives symbols, strips, applies Linux
  cosign, and stages the helper bundle. Separate
  [macOS jobs](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/.github/workflows/rust-release.yml#L496-L612)
  sign and notarize binaries. ELF hardening properties in this ledger come from released-artifact
  inspection, not this workflow range. Class: `verified/source`.
- [Release gate dependencies](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/.github/workflows/rust-release.yml#L1109-L1124)
  do not make the normal test jobs direct release dependencies. The exact tag had one successful
  `rust-release` run and 44 attached checks, mostly release work. Class: `verified/source` and
  `verified/local` GitHub API observation.
- [Bazel macOS/Linux matrix](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/.github/workflows/bazel.yml#L17-L108)
  exercises macOS and Linux GNU/musl, while separate
  [Windows shards](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/.github/workflows/bazel.yml#L133-L220)
  run on Windows hosts. Class: `verified/source`, not exact-tag execution proof.
- Cargo release commands do not consistently use `--locked`, and the committed release lockfile
  stores workspace packages as `0.0.0`, causing Cargo to rewrite 132 entries locally. No external
  dependency change was observed. Class: `verified/source` and `verified/local`; reproducible-build
  impact remains `unverified`.
- The repository has broad test inventory, but its checked-in numeric performance benchmark is
  limited to prompt-image work and CI runs it as smoke without a regression threshold. No tracked
  app-server startup, throughput, queue saturation, remote reconnect, or TUI latency budget was
  found. Class: `verified/source` inventory limitation.

## Targeted Source Tests

The release workspace required `just` 1.56.0 and `cargo-nextest` 0.9.140. The audit environment also
lacked `pkg-config` and OpenSSL development headers; exact Ubuntu 24.04 packages were downloaded and
extracted under `/tmp` without changing the system. Starting with Rust/Cargo 1.95.0 and the exact
source checkout, the relevant clean-shell setup and final commands were:

```bash
TOOLS=/tmp/akra-upstream-codex-tools
DEPS=/tmp/akra-upstream-codex-build-deps
mkdir -p "$TOOLS" "$DEPS/debs" "$DEPS/root"
cargo install --locked --root "$TOOLS" --version 1.56.0 just
cargo install --locked --root "$TOOLS" --version 0.9.140 cargo-nextest

cd "$DEPS/debs"
apt-get download \
  pkgconf=1.8.1-2build1 \
  pkgconf-bin=1.8.1-2build1 \
  libpkgconf3=1.8.1-2build1 \
  libssl-dev=3.0.13-0ubuntu3.11 \
  libssl3t64=3.0.13-0ubuntu3.11
for package in ./*.deb; do
  dpkg-deb --extract "$package" "$DEPS/root"
done

export PATH="$TOOLS/bin:$DEPS/root/usr/bin:$HOME/.cargo/bin:$PATH"
export LD_LIBRARY_PATH="$DEPS/root/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PKG_CONFIG_SYSROOT_DIR="$DEPS/root"
export PKG_CONFIG_LIBDIR="$DEPS/root/usr/lib/x86_64-linux-gnu/pkgconfig:$DEPS/root/usr/lib/pkgconfig:$DEPS/root/usr/share/pkgconfig"
export CFLAGS="${CFLAGS:+$CFLAGS }-I$DEPS/root/usr/include/x86_64-linux-gnu"

cd /tmp/akra-upstream-codex-v0.144.1-source/codex-rs
just test -p codex-app-server-protocol
just test -p codex-app-server-client
```

Observed final results:

```text
codex-app-server-protocol: 256 passed, 0 skipped
codex-app-server-client:    27 passed, 0 skipped
```

The client suite included typed and remote request round trips, auth-token transport policy,
WebSocket and Unix-socket operation, server-request resolution, disconnect events, backpressure,
lag markers, shutdown, and session-source behavior. The protocol suite included stable filtering,
experimental markers, serialization, schema fixtures, thread paging, sandbox shapes, and applied
thread/turn types. Class: `verified/local`.

This is targeted evidence, not a full source-suite pass. The first build attempts failed on missing
local tools/development headers and are environment setup failures, not product test failures.

## Direct App-Server Performance Probe

### Boundary

Each sample:

1. used the released `codex-app-server-x86_64-unknown-linux-musl` binary;
2. created a fresh isolated `CODEX_HOME`;
3. spawned the default stdio app-server with plugin startup tasks enabled;
4. sent initialize and initialized;
5. requested `account/read` and `thread/list`;
6. measured spawn-to-final-response elapsed time;
7. sampled the recursively enumerated process tree's RSS from spawn through the final response;
8. exited the process.

The binary and filesystem page cache were warm. There was no TTY, rendering, authentication, model
call, explicit MCP/plugin request, stream, Akra process, or parallel worker. Default plugin startup
was not disabled. Manual inspection of this path observed app-server launch
`git ls-remote https://github.com/openai/plugins.git HEAD` plus Git transport helpers. The response
clock did not wait for that public-network background work to finish. Ten runs were performed on the
snapshot environment above.

This default behavior matches the app-server
[startup hook](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server/src/message_processor.rs#L428-L438)
and the enabled-plugin path that
[starts curated repository sync](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core-plugins/src/manager.rs#L1926-L1937).
Class: `verified/source` plus `verified/local` process-tree observation.

### Raw results

| Run | Elapsed ms | Sampled peak RSS KiB | Process count |
| ---: | ---: | ---: | ---: |
| 1 | 120.781 | 82,440 | 4 |
| 2 | 122.712 | 90,524 | 4 |
| 3 | 123.812 | 93,328 | 4 |
| 4 | 120.261 | 92,052 | 4 |
| 5 | 117.292 | 87,676 | 4 |
| 6 | 120.708 | 88,272 | 4 |
| 7 | 122.124 | 88,352 | 4 |
| 8 | 120.605 | 88,336 | 4 |
| 9 | 119.439 | 91,224 | 4 |
| 10 | 120.829 | 87,920 | 4 |
| min | 117.292 | 82,440 | 4 |
| median | 120.745 | 88,344 | 4 |
| max | 123.812 | 93,328 | 4 |

The checked-in [benchmark harness](scripts/benchmark-app-server.mjs) defines the clock boundary and
uses the [shared Linux sampler](scripts/app-server-harness.mjs) to union every thread's `/proc`
children list recursively and poll process `VmRSS` every requested 2 ms. The sampler fails closed
when no root-task `children` inventory is readable, rather than reporting root-only RSS as a complete
tree. Reproduce the one omitted warmup plus ten recorded runs with:

```bash
APP_SERVER_BIN=/tmp/akra-upstream-codex-v0.144.1-bin/codex-app-server-x86_64-unknown-linux-musl
printf '%s  %s\n' \
  b68de8340b2ccb8aeb23b6f33a4b4dff4f203ff84f6d82b6feedf334aba9a4fb \
  "$APP_SERVER_BIN" | sha256sum --check --strict -
CODEX_APP_SERVER="$APP_SERVER_BIN" \
  node docs/competitive/upstream-codex/scripts/benchmark-app-server.mjs
```

Event-loop scheduling can delay a poll and RSS sampling can miss a shorter peak. This result remains
`verified/local` orientation and must not be used as a release regression gate. The default plugin
warmup and unfinished network work make it neither an isolated protocol-cost sample nor a substitute
for a separately controlled plugin-disabled stratum. It is a default-path lower-bound input to the
existing Akra Native Performance Evidence Contract, not a comparative interactive result.

## Doctor Probe

In a fresh isolated home, `codex doctor --json` returned structured redacted check results. App-server,
config, Git, state, and sandbox checks executed. Credential, terminal, and installation-match checks
failed as expected for the isolated/noninteractive/mismatched-artifact setup. The command also
attempted reachability work. Class: `verified/local`.

The probe supports considering doctor output as bounded diagnostic input. It does not establish a
stable API, offline behavior, low latency, or safety for automatic periodic Admin polling.

## Akra Baseline Evidence

All links below use Akra commit `226e4794b84107704378ecc1ea65f7d5c27750e5`.

### Protocol projection

- [Notification classification](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/protocol/contract_tests.rs#L21-L99)
  lists 8 handled, 13 deferred, 29 diagnostic-only, and 18 ignored methods. Class:
  `verified/source`.
- [Active reducer](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/protocol/turn_notifications.rs#L201-L253)
  drops deferred methods without typed translations, terminates every `error`, and converts every
  matching `turn/completed` to generic completion while discarding the terminal event send result.
  The [connection wait loop](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/connection.rs#L1204-L1304)
  then returns `Result<()>` rather than a typed terminal receipt. Class: `verified/source`.
- [Completed item projection](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/protocol/turn_notifications.rs#L485-L533)
  handles only agent message, file change, and command execution live; the
  [snapshot projection](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/protocol/turn_notifications.rs#L257-L300)
  adds user messages. Class: `verified/source`.
- [Core turn reducer](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/core/app/turn_stream.rs#L122-L139)
  has generic completed and failed terminal branches but no upstream interrupted/retrying status.
  Class: `verified/source`.
- [Parallel archive path](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/mod.rs#L1333-L1357)
  archives when the stream result is `Ok`; [prompt-log status](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/mod.rs#L982-L1023)
  similarly derives completed from `Ok`. Class: `verified/source`.

### Capability and sessions

- [Initialize capability](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/connection.rs#L1048-L1066)
  sends `experimentalApi: false`. Class: `verified/source`.
- [Thread response DTO](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/protocol.rs#L482-L536)
  reads only `thread` from start/resume and projects a subset of thread provenance. Class:
  `verified/source`.
- [Thread list params](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/protocol.rs#L270-L283)
  omit cursor despite retaining response `nextCursor`. Class: `verified/source`.
- [Static model UI](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/inbound/tui/app/model_selection_overlay_ui.rs#L31-L74)
  and [closed effort type](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/domain/conversation.rs#L96-L116)
  do not consume `model/list` or preserve arbitrary official effort values. Class:
  `verified/source`.

### Approval and secret boundaries

- [Approval parser](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/approval.rs#L90-L132)
  requires a command for command approval, which loses official network-only requests that can omit
  command/cwd. Class: `verified/source`.
- `serverRequest/resolved` is diagnostic-only in the classification link above. It is not available
  as typed approval state. Class: `verified/source`.
- [Child process environment policy](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/connection.rs#L356-L398)
  defaults to scrubbed and accepts exact `all` as an elevated override. The
  [allowlist/filter](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/connection.rs#L443-L517)
  filters ambient credential-bearing values before app-server launch. Class: `verified/source`.
- [Generated-shell launch policy](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/connection.rs#L668-L768)
  defaults inheritance to `core`, preserves default secret excludes, and disables login-shell
  processing through explicit Codex overrides. Class: `verified/source`.
- [Execution policy default](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/execution_policy.rs#L37-L57)
  supplies workspace-write with user/on-request approval to normal work turns without consulting an
  official project trust decision. Thread setup and
  [hidden planning turns](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/mod.rs#L628-L649)
  use read-only. The normal-work default is more permissive than the first-party no-decision
  read-only default, even though Akra's environment inheritance is narrower. Class:
  `verified/source`.
- The OpenCode [Known-Canary Bounded Egress Matrix](../opencode/gap-matrix.md#known-canary-bounded-egress-matrix)
  already owns end-to-end proof for that boundary. Upstream's `inherit: All` default strengthens
  its priority; it does not justify a duplicate secret subsystem.

### Delivery and packaging

- [Current contract](../../supersession/current-contract.md) defines accepted planning, parallel
  worktree/lease, review, integration, and cleanup behavior. Class: `verified/source` with the
  normal implementation caveat.
- [Native packaging runbook](../../plan/13-native-packaging-and-operator-runbook.md) and
  `scripts/package_native_release.sh` already require locked Cargo builds, bundle-internal
  checksums, archive checksum, safe extraction validation, and npm provenance. Class:
  `verified/source`.
- The concurrently active `fix/native-validation-evidence-contract` lane was not inspected as
  baseline truth and was not modified. Its eventual merge may supersede release-evidence gaps from
  this audit.

## Audit Limits

The audit did not:

- authenticate a model call or compare model quality;
- run a fair interactive upstream-TUI versus Akra-TUI benchmark;
- run the full Codex source suite or all Bazel/Cargo CI matrices;
- independently verify cosign bundles, SBOM/provenance, macOS notarization, Windows signing, or
  arm64 artifacts;
- exercise remote-control relay, remote exec Noise transport, realtime, cloud tasks, authenticated
  apps/plugins/MCP, or paid service paths;
- reproduce the inferred core-event backlog, V2 limiter race, side-thread peak accumulation,
  rollout permission exposure, or large-input compaction failure;
- test sandbox behavior on a host with the complete package archive, system `bwrap`, macOS, or
  Windows;
- promote the checked-in audit harnesses or their local output into a trusted release receipt;
- claim that feature stage, protocol stability, default enablement, and end-user maturity are the
  same property;
- modify or clean the shared temporary source lockfile after concurrent Cargo work changed it.

Performance conclusions remain intentionally narrow. Security findings describe exact boundaries
and defaults; they are not a general claim that upstream or Akra is secure or insecure. The source
inventory is large and fast-moving, so absence findings are bounded to the pinned commit and paths
searched.
