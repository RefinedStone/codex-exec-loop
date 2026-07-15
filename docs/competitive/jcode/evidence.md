# jcode Evidence Ledger

This ledger separates inspected source from vendor claims and proposed design. The corresponding
product conclusions live in [analysis.md](analysis.md), and Akra decisions live in
[gap-matrix.md](gap-matrix.md).

## Snapshot

| Field | Value |
| --- | --- |
| Product | jcode |
| Official repository | <https://github.com/1jehuang/jcode> |
| Release | [v0.43.0](https://github.com/1jehuang/jcode/releases/tag/v0.43.0) |
| Peeled release commit | `649276753ae11948759192c067dfc4c90fafd47f` |
| Release published | 2026-07-11 06:00:50 UTC |
| Audit date | 2026-07-12 (Asia/Seoul) |
| Akra baseline | `66333152170124a42aca6f49ed2f72fa6a8293d7` on `prerelease` |
| Akra version | 1.3.5 |
| Auditor environment | Linux x86_64, source inspection and static commands |

The jcode checkout was a clean detached checkout of the release tag. The Akra baseline was a clean
worktree from `origin/prerelease`.

## Reproduction Commands

```bash
git clone --depth 1 --branch v0.43.0 \
  https://github.com/1jehuang/jcode.git /tmp/akra-jcode-v043-audit
git -C /tmp/akra-jcode-v043-audit rev-parse HEAD
git -C /tmp/akra-jcode-v043-audit describe --tags --exact-match
git -C /tmp/akra-jcode-v043-audit log -1 --format='%cI %s'
```

Observed:

```text
649276753ae11948759192c067dfc4c90fafd47f
v0.43.0
2026-07-10T22:20:40-07:00 release: v0.43.0
```

Historical snapshot:

```bash
git clone --depth 1 --branch v0.11.2 \
  https://github.com/1jehuang/jcode.git /tmp/akra-jcode-v0.11.2-audit
git -C /tmp/akra-jcode-v0.11.2-audit rev-parse HEAD
```

Observed: `7e419c81a69936336c8ee9df6287f4919708d5c1`.

Inventory commands:

```bash
JCODE=/tmp/akra-jcode-v043-audit
AKRA=/path/to/codex-exec-loop-worktree

count_root_and_workspace_crates() {
  local workspace_crates=0
  if [ -d crates ]; then
    workspace_crates="$(find crates -mindepth 2 -maxdepth 2 -name Cargo.toml | wc -l)"
  fi
  printf '%s\n' "$((1 + workspace_crates))"
}

cd "$JCODE"
git ls-files | wc -l
git ls-files '*.rs' | wc -l
git ls-files '*.rs' | xargs wc -l | tail -1
count_root_and_workspace_crates
find src crates -type f -name '*.rs' \
  ! -path '*/tests/*' ! -name '*_test.rs' ! -name '*_tests.rs' \
  -print0 | xargs -0 wc -l | tail -1
rg -n '#\[(tokio::)?test\]' --glob '*.rs' . | wc -l
rg -n '#\[(tokio::)?test\]' --glob '*.rs' \
  crates/jcode-tui crates/jcode-tui-* crates/jcode-render-core | wc -l
git ls-files '*.rs' | awk '/^src\//' | wc -l
git ls-files '*.rs' \
  | awk '/^(src\/tui\/|crates\/jcode-tui[^/]*\/|crates\/jcode-render-core\/)/' \
  | xargs wc -l | tail -1
git ls-files '*.rs' \
  | awk '/^(src\/desktop\/|crates\/jcode-desktop[^/]*\/)/' \
  | xargs wc -l | tail -1

cd "$AKRA"
git ls-files | wc -l
git ls-files '*.rs' | wc -l
git ls-files '*.rs' | xargs wc -l | tail -1
count_root_and_workspace_crates
find src -type f -name '*.rs' \
  ! -path '*/tests/*' ! -name '*_test.rs' ! -name '*_tests.rs' \
  -print0 | xargs -0 wc -l | tail -1
rg -n '#\[(tokio::)?test\]' --glob '*.rs' . | wc -l
```

The counts are orientation data, not quality scores. `production Rust LOC` excludes obvious test
paths and test-suffixed files but can still include test modules inside production files.

| Inventory | jcode v0.43.0 | Akra baseline |
| --- | ---: | ---: |
| Tracked files | 1,543 | 645 |
| Rust files | 1,084 | 457 |
| Root plus workspace crates | 77 | 1 |
| All Rust LOC | 631,544 | 235,156 |
| Approximate production Rust LOC | 517,632 | 204,541 |
| Repository-wide Rust `#[test]` / `#[tokio::test]` markers | 6,205 | 2,239 |
| TUI-family test markers | 2,197 | not separately counted |

### Evolution From The Prior Internal Snapshot

Akra's deleted 2026-04 analysis used jcode v0.11.2. A second clean checkout pinned that tag at
`7e419c81a69936336c8ee9df6287f4919708d5c1`. Using the same path/count definitions on both tags:

| Inventory | v0.11.2 | v0.43.0 | Change |
| --- | ---: | ---: | ---: |
| Rust files | 735 | 1,084 | +47% |
| All Rust LOC | 336,208 | 631,544 | +88% |
| Root plus workspace crates | 35 | 77 | +120% |
| Root `src` Rust files | 631 | 45 | -93% |
| TUI-family Rust LOC | 119,951 | 213,305 | +78% |
| Desktop-family Rust LOC | 11,880 | 71,788 | +504% |

`TUI-family` means tracked Rust paths under `src/tui/`, `crates/jcode-tui*`, or
`crates/jcode-render-core`. `Desktop-family` means `src/desktop/` or `crates/jcode-desktop*`.

This verifies both real modular extraction and rapid scope growth. Root source moved into crates,
but the product did not become smaller. The largest dependency spine still flows through
`jcode-tui -> jcode-app-core -> jcode-base`, and all three remain large compilation and ownership
units.

## Immutable Source Ledger

### Product And Release

- [Release v0.43.0](https://github.com/1jehuang/jcode/releases/tag/v0.43.0) identifies the release
  commit and lists terminal math, asynchronous swarm waits, remote working-directory override, and
  idle TUI heap release changes. Class: `documented`.
- [README product position](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/README.md#L12-L13)
  presents multi-session workflows, customizability, and performance as the product thesis. Class:
  `documented`.
- [Cargo workspace](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/Cargo.toml#L1-L91)
  declares v0.43.0 and the runtime, provider, protocol, TUI, desktop, storage, memory, and tool
  crates. Class: `verified`.

### Runtime Architecture

- [Server architecture](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/SERVER_ARCHITECTURE.md#L9-L110)
  describes one daemon owning sessions and state, clients over a user socket, reconnect, reload,
  provider state, and a shared MCP pool. Source modules and the protocol crate support the topology.
  Class: `verified` for topology, `documented` for operational guarantees not reproduced here.
- [Protocol crate](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/crates/jcode-protocol/src/lib.rs#L1-L22)
  defines newline-delimited JSON over a local socket and separate main and agent communication.
  Class: `verified`.
- [Protocol agent snapshot](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/crates/jcode-protocol/src/lib.rs#L202-L273)
  carries touched files, lifecycle status, completion report, attachments, activity age, current
  tool, provider/model, token churn, and todo progress. Class: `verified`.
- [Multi-session client architecture](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/MULTI_SESSION_CLIENT_ARCHITECTURE.md#L1-L34)
  explicitly marks the many-session-in-one-client workspace as proposed. The currently documented
  model remains one server with many mostly single-session clients. Class: `proposed` for the
  workspace direction and `documented` for the current client model.

### TUI And Visual System

- [README UI section](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/README.md#L287-L300)
  claims side panels, Mermaid, negative-space widgets, high render throughput, custom scrollback,
  and alignment modes. Class: `documented`; throughput is `unverified`.
- [Info widget inventory](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/crates/jcode-tui/src/tui/info_widget.rs#L69-L183)
  implements explicit kinds, priority, side preference, and minimum height for workspace, todos,
  context, memory, swarm, usage, model, diagrams, ambient work, and git. Class: `verified`.
- [Side-panel tool](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/crates/jcode-app-core/src/tool/side_panel.rs#L11-L153)
  exposes status, write, append, load, focus, and delete actions to the agent runtime. Class:
  `verified`.
- [Mermaid terminal crate](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/crates/jcode-tui-mermaid/src/lib.rs#L209-L272)
  exposes cached, deferred, stable-fit, and viewport rendering paths, while the
  [deferred-render state](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/crates/jcode-tui-mermaid/src/lib.rs#L415-L464)
  owns the shared background queue. The README's `1800x` claim was not reproduced. Class:
  `verified` for code, `unverified` for the ratio.
- TUI-focused source contains 2,197 Rust test markers under the audited counting rule, including
  input/copy, remote reload, startup input, scroll/copy, smoothness, swarm plan, onboarding, and
  visual paths. Class: `verified`.

### Performance

- [README performance tables](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/README.md#L48-L248)
  publish 27.8 MB PSS with local embedding disabled, 14.0 ms to first frame, 48.7 ms to first input,
  and multi-client comparisons. Class: `unverified` in this audit.
- [Visible-ready benchmark](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/scripts/bench_startup_visible_ready.py#L1-L330)
  drives an 80x24 PTY, answers terminal capability queries, renders with `pyte`, sends a probe after
  meaningful content, defaults to ten runs, and can emit JSON. Class: `verified`.
- [Memory benchmark](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/scripts/bench_memory_cli.py#L1-L432)
  defines process/session launch specs and records versions and PSS. Class: `verified`.
- The README identifies compared versions only for the memory rerun, and that list uses jcode
  `v0.9.1888-dev`, not v0.43.0. It states only "this Linux machine" for startup. No raw startup or
  PSS JSON artifact, full hardware stamp, or current-tag rerun was found in tracked files. Class:
  `verified` source-inventory limitation.

The scripts make the claims more credible than an unauditable marketing table, but they do not make
the published numbers directly comparable to Akra. Akra includes a TUI process plus an official
`codex app-server` child boundary, so any comparison must count the complete process tree and split
first frame, input echo, app-server ready, first stream delta, and parallel-worker memory.

### Swarm And Planning

- [README swarm section](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/README.md#L304-L318)
  claims same-repository agents, read/change notification, direct and broadcast messaging, conflict
  resolution, and autonomous spawning. Class: `documented`.
- [Conflict-handling design](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/SWARM_ARCHITECTURE.md#L295-L313)
  says coordination is optimistic and lock-free: file-touch notifications detect risk, then the
  involved agents communicate through a DM or channel. This does not prove the README's stronger
  statement that all conflicts are automatically resolved. Class: `documented` conflict; the
  conclusion that notification is not deterministic resolution is `inferred`, and automatic
  resolution remains `unverified`.
- [Swarm architecture status](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/SWARM_ARCHITECTURE.md#L1-L26)
  marks the agent-first design largely implemented while pointing to a DAG-first migration.
  Class: `documented`.
- [Recursive ownership and worktree roles](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/SWARM_ARCHITECTURE.md#L27-L94)
  define recursive spawning, report-back ownership, a root plan coordinator, optional worktree
  managers, and agents. Source inspection found generic role strings but did not identify an
  automated git-worktree creation, merge, or cleanup lifecycle. Treat the worktree manager as a
  `documented` agent-enacted role over optional worktrees; automated isolation/integration remains
  `unverified`.
- [DAG design status](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/SWARM_TASK_GRAPH.md#L1-L16)
  says the full reframe is being implemented but explicitly marks the DAG engine, deep/light modes,
  gates, graph growth, artifact dataflow, and subtree broadcast live. Channel/shared-context removal
  remains pending. Class: `documented` for the named live implementation and `proposed` for the
  unfinished migration.
- [Deep-mode scale](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/SWARM_TASK_GRAPH.md#L54-L91)
  specifies unbounded recursion and fan-out up to a 1,000-member cap. The
  [source constant](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/crates/jcode-swarm-core/src/lib.rs#L60-L63)
  implements that cap. This audit found unit-level cap evidence but no reproduced 1,000-agent live
  load result. Class: `verified` configuration, `unverified` operational scale.
- [Plan graph protocol](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/crates/jcode-protocol/src/lib.rs#L311-L367)
  exposes ready, blocked, active, completed, failed, cycle, unresolved dependency, confidence, and
  growth state. Class: `verified`.

### Memory, Providers, And Extensibility

- [Memory architecture status](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/MEMORY_ARCHITECTURE.md#L1-L20)
  marks core memory implemented and the graph-based hybrid planned. Class: `documented` with source
  support for memory types, extraction, retrieval, and UI activity.
- [README provider inventory](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/README.md#L322-L365)
  lists subscription-backed OAuth, direct providers, local endpoints, OpenAI-compatible profiles,
  and MCP configuration. Provider and runtime crates corroborate the breadth. Class: `verified` for
  code inventory, `documented` for live provider behavior not exercised here.
- [Lifecycle hooks](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/HOOKS.md#L1-L87)
  define observer hooks and a synchronous pre-tool gate. Nonzero errors other than the explicit
  block code fail open by design. Class: `documented`.

### Desktop And Remote Control

- [Desktop crate](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/crates/jcode-desktop/Cargo.toml#L1-L26)
  is a private v0.1.0 Rust package using `winit`, `wgpu`, and custom rendering dependencies. Class:
  `verified`.
- [Desktop architecture](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/DESKTOP_APP_ARCHITECTURE.md#L1-L55)
  is explicitly proposed, while later implementation notes and source show a substantial prototype.
  This is not evidence of a supported public desktop release. Class: `proposed`, with a `verified`
  source prototype.
- No general operational Admin site equivalent to Akra's Axum/Askama surface was identified. The
  jcode source instead emphasizes TUI clients, a custom desktop client, gateway/mobile directions,
  and daemon/debug interfaces. Class: `verified` inspected inventory, not proof that an external
  service does not exist.

### Quality And Maintainability

- [CI workflow](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/.github/workflows/ci.yml#L1-L320)
  runs formatting, all-target/all-feature checks, clippy, warning/size/panic/error/dependency
  ratchets, unused dependency checks, and platform build/test jobs. Class: `verified` configuration.
- [Code-size ratchet](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/scripts/check_code_size_budget.py#L1-L27)
  blocks new production Rust files over 1,200 LOC and growth in tracked oversized files. Class:
  `verified`.
- At the clean v0.43.0 tag, `check_code_size_budget.py`, `check_test_size_budget.py`,
  `check_panic_budget.py`, and `check_swallowed_error_budget.py` all exited 1. The code-size check
  reported 52 regressions, including a new oversized file and growth in server, provider, swarm,
  and TUI files. `check_dependency_boundaries.py` and `check_wildcard_reexport_budget.py` passed.
  The exact commands and captured stdout/stderr are checked in as
  [guard-results.txt](guard-results.txt). Class: `verified` local output.

This means jcode has unusually serious quality instrumentation, but the tagged source did not
satisfy four of its own checked-in ratchets during this audit. A clean v0.11.2 checkout also failed
the four ratchets that existed there, which indicates persistent baseline/release drift rather than
a v0.43-only regression. The v0.11.2 commands and output are captured in
[guard-results-v0112.txt](guard-results-v0112.txt). The persistent-drift conclusion is `inferred`
from the two `verified` snapshots. The presence of a guard is not the same as a green release gate.

## Akra Baseline Evidence

The comparison read current Akra source and contracts at the pinned commit. Local links make the
files easy to open; the adjacent immutable links preserve snapshot evidence.

- [current product state](../../reference/current-product.md) ([pinned](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/docs/design/01-current-product-state.md))
- [docs surface map](../../README.md) ([pinned](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/docs/README.md))
- [current operator contract](../../reference/current-product.md) ([pinned](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/docs/supersession/current-contract.md))
- [TUI architecture](../../design/07-tui-layered-architecture-and-aesthetic-contract.md) ([pinned](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/docs/design/07-tui-layered-architecture-and-aesthetic-contract.md))
- [runtime architecture](../../reference/architecture.md) ([pinned](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/docs/design/05-parallel-control-plane-architecture.md))
- [app-server adapter](../../../src/adapter/outbound/app_server/mod.rs) ([pinned](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/outbound/app_server/mod.rs))
- [parallel services](../../../src/application/service/parallel_mode/mod.rs) ([pinned](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/application/service/parallel_mode/mod.rs))
- [Admin inbound adapter](../../../src/adapter/inbound/admin_api/mod.rs) ([pinned](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/inbound/admin_api/mod.rs))

Gap-specific source evidence:

- the pinned app-server schema includes `turn/steer` and `expectedTurnId`, while the
  [interactive runtime port](../../../src/application/port/outbound/interactive_turn_runtime_port.rs)
  ([pinned schema](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/schema/codex_app_server_protocol.v2.schemas.json#L21238-L21284),
  [pinned port](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/application/port/outbound/interactive_turn_runtime_port.rs))
  exposes load, stop, approval, start-thread, and start-turn operations but no steering method;
- [turn submission](../../../src/adapter/inbound/tui/app/turn_submission_runtime.rs) accepts manual
  input only from a ready conversation
  ([pinned](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/inbound/tui/app/turn_submission_runtime.rs));
- [protocol contract tests](../../../src/adapter/outbound/app_server/protocol/contract_tests.rs)
  classify command output, patch update, item start, turn diff, and plan update as deferred, with
  token usage diagnostic-only
  ([pinned](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/outbound/app_server/protocol/contract_tests.rs#L21-L78));
- completed command and file-change items already become typed tool activity in
  [turn notification parsing](../../../src/adapter/outbound/app_server/protocol/turn_notifications.rs),
  accumulate as current/last-turn counts and a latest summary in
  [turn activity state](../../../src/adapter/inbound/tui/app/conversation_model/turn_activity.rs), and
  receive a running-turn priority line in
  [the shared tail](../../../src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_shared.rs)
  ([pinned parsing](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/outbound/app_server/protocol/turn_notifications.rs#L485-L533),
  [pinned state](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/inbound/tui/app/conversation_model/turn_activity.rs#L8-L49),
  [pinned tail](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_shared.rs#L74-L127));
- the runtime distributor defaults to the reviewed PR gate, while an exact parent-level autonomous
  opt-in skips approval, clean-merge, and required-check gates even with a PR; eligible PR modes may
  also skip PR automation before direct integration
  ([pinned contract](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/docs/supersession/current-contract.md#L124-L141),
  [pinned policy](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/application/service/parallel_mode/distributor/delivery/github.rs#L177-L256),
  [pinned gates](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/application/service/parallel_mode/distributor/delivery/github.rs#L655-L719));
- [the app-server connection](../../../src/adapter/outbound/app_server/connection.rs) terminates its
  child on transport failure and drop, while
  [shared runtime reconnect](../../../src/adapter/outbound/app_server/runtime.rs) launches a new child;
  existing-thread submission resumes the thread and starts a new turn rather than reattaching to an
  already-running prior turn
  ([pinned termination](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/outbound/app_server/connection.rs#L2438-L2462),
  [pinned reconnect](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/outbound/app_server/runtime.rs#L27-L58),
  [pinned resumed turn](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/outbound/app_server/mod.rs#L1228-L1275));
- the distributor queue freezes its base and source tip, and default PR readiness binds the PR head,
  approval, clean-merge state, and required checks to that frozen source
  ([pinned queue record](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/application/port/outbound/planning_authority_port.rs#L115-L133),
  [pinned readiness](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/application/service/parallel_mode/distributor/delivery/github.rs#L675-L719));
- the current parallel `validation_summary` is derived from changed planning-file paths, with a
  fallback stating that validation was not reported; it is not command/exit/artifact proof
  ([pinned producer](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/application/service/parallel_mode/orchestrator_loop.rs#L1196-L1209),
  [pinned summary](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/application/service/parallel_mode/orchestrator_loop.rs#L1413-L1426),
  [pinned fallback](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/application/service/parallel_mode/completion.rs#L163-L168));
- [Admin dashboard mapping](../../../src/adapter/inbound/admin_api/akra_dashboard.rs) leaves success,
  throughput, test success, error rate, and selected-task percentage explicitly uncollected
  ([pinned](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/inbound/admin_api/akra_dashboard.rs)).

The supporting product-state document says the inline shell is the only frontend, while the code
and docs map include Admin, CLI, Telegram, and automation. The comparison therefore uses the source
surface map rather than repeating that stale sentence.

## Audit Limits

- jcode was not built or launched in this audit.
- No jcode terminal screenshot, input trace, provider request, memory retrieval, swarm run, or
  desktop session was reproduced.
- Published performance numbers were not rerun and are not accepted as current-tag measurements.
- Code and tests were inspected, but source presence does not prove default enablement or release
  maturity.
- Several jcode design documents explicitly mix implemented, migrating, proposed, and design-only
  states. The analysis preserves those distinctions.
- Akra has no repeatable startup/input/PSS benchmark at the pinned baseline, so no performance winner
  is declared.
