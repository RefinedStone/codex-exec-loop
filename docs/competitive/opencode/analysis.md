# OpenCode v1.17.18 Deep Dive

[한국어 번역](../../ko/competitive/opencode/analysis.md)

This analysis compares OpenCode v1.17.18 with Akra at prerelease commit
`354f4782a73ffabab6abf342cb0f37285a96c177`. Immutable source links, reproduced commands, release
artifact identity, and audit limits are in [evidence.md](evidence.md). Akra-relative decisions and
reviewable implementation slices are in [gap-matrix.md](gap-matrix.md).

## Executive Verdict

OpenCode is a broad coding-agent platform with a strong terminal product, not a Codex wrapper. It
owns provider routing, model execution, tools, permissions, MCP, LSP, plugins, sessions, persistence,
server APIs, web and Electron clients, GitHub automation, and an experimental second-generation
runtime that is already mounted but still incomplete. Its reason to choose it is breadth joined to
a polished, attachable TUI: one installation
can operate many providers and projects through terminal, browser, desktop, SDK, and automation
surfaces.

The strongest threats to Akra are:

1. server-scoped asynchronous turns plus external attach when a separately started server remains
   alive after an attach client detaches, and shared web/TUI access to that server;
2. a protocol-rich TUI with session browsing, diff/timeline workflow, child navigation, permission/
   question forms, command discovery, and compact terminal mode;
3. GitHub event and schedule entry points that can run an agent and create a commit, branch, and pull
   request;
4. a broad shared API and application UI reused across hosted web, embedded web, and six desktop
   release targets.

Those advantages do not prove a performance, durability, or safety winner. The audit found no fair
same-machine authenticated interactive benchmark. OpenCode's manual renderer performance suite has
no machine-independent budgets and excludes the packaged Electron process tree. Its live external
event stream has no usable replay cursor, legacy background-job ownership is process-local, and its
GitHub path does not provide Akra's reviewed frozen-range integration authority.

The most consequential exploitable weakness is the trust boundary. In a reproduced isolated
server, no password meant no authentication and unauthenticated `GET /config` returned resolved
`openai.options.apiKey` and disabled local-MCP `environment` canaries supplied through configuration;
legacy `GET /provider` returned the provider canary. `{file:...}`, provider auth files, remote MCP
headers/OAuth, actual MCP spawn, GitHub tokens, and v2 configured-secret output were not dynamically
tested. Source inspection also found a shell path that does not compose the direct-read `.env`
prompt, and a GitHub automation path that writes an active app token into repository-local Git
configuration and can leave that header behind. These are bounded findings, not a claim that every
default TUI session is remotely exposed: the default TUI uses an in-process transport, and
standalone servers default to loopback unless the operator changes the network configuration.

Akra should not answer by cloning OpenCode's provider runtime, plugin host, browser IDE, or optional
authentication model. It should sharpen a narrower product:

> Akra is the Codex-first operating and delivery layer that turns official `codex app-server`
> sessions into durable, inspectable, review-delivered work.

That first requires closing Akra's live execution, steering/recovery, measurable performance, and
exact-source delivery-evidence gaps. Approval choice fidelity follows as a bounded protocol slice;
fork lineage and command discovery remain later session/TUI work. Existing delivery, authenticated
Admin, and default-scrubbed child-environment boundaries must remain visible and canary-tested
invariants rather than a replacement product thesis.

## Product And Audience

OpenCode targets developers who want one agent harness across model providers and surfaces. The
audited release provides:

- an alternate-screen TUI and a compact native-scrollback mode;
- headless `run`, long-lived `serve`, browser-opening `web`, and external `attach` commands;
- session export/import, statistics, provider and agent management, MCP, plugins, ACP, GitHub, and
  pull-request commands;
- a shared Solid application used by hosted web, embedded server UI, and Electron desktop;
- an SDK/OpenAPI surface and an experimental v2 protocol/runtime;
- a VS Code-family extension that launches the terminal client and appends current editor context;
- GitHub issue, pull request, review-comment, schedule, and manual-dispatch automation.

This breadth is a real product advantage. It is also a different product thesis from Akra. Akra's
operator chooses official Codex behavior and durable delivery authority, not provider portability or
an arbitrary extension host.

## Architecture And Runtime Authority

### Current OpenCode Path

The legacy/current path has one server authority for sessions, providers, tools, configuration,
files, PTYs, permissions, sharing, and events. The normal TUI spawns a worker and uses an in-process
fetch transport plus a direct event bridge. External attach uses HTTP and SSE. `serve`, `web`, the
hosted application, Electron renderer, extension, and generated SDK all reuse the server boundary in
different ways.

This topology can separate client lifetime from turn lifetime. A prompt can run asynchronously in
server scope while the separate `attach` command connects later to an independently running
`serve` process. The default `opencode` TUI does not provide that survival contract: it owns a worker
server and shuts the worker down on exit. Source verifies the topology; this audit did not execute
an end-to-end detach experiment. It is not evidence of crash-safe execution because the current run
registry is in memory and instance disposal cancels active work.

The external event path also has a continuity hole. The server emits events without replayable SSE
IDs. The TUI retries with bounded backoff but does not re-bootstrap a complete snapshot after a
disconnect. The browser application performs a broader root/directory bootstrap and refreshes
session lists and status after reconnect, but it does not force every cached session message/part
window to reload. It is therefore reasonable to infer that either external client can retain a
stale transcript window across an event gap, while the browser repairs more catalog/status state;
the default in-process TUI path is not affected by a network disconnect in the same way.

Headless `opencode run` also derives stdout and idle completion from the live event stream and does
not use the successful prompt response body to reconstruct missing output. Combined with the
legacy unbounded event queue and absent replay IDs, disconnect can plausibly produce incomplete
stdout or abnormal completion waiting. That effect is source-supported inference, not a reproduced
failure.

### Mounted v2 Path And Maturity

The shipped listener mounts a second protocol and server route layer beside the legacy APIs. Its
handlers, durable event/projector storage, process-local run coordinator, and per-session history/
replay routes are therefore source-verified shipped code, even where endpoint names or source
comments label the interface experimental. Evidence status and product maturity are separate axes.

Important ownership, retry, cancellation, plugin-parity, stale-work, and crash-continuation behavior
is still explicitly incomplete. The mounted v2 provider response also carries raw request headers
and body; v1 configuration lowering can place `Authorization`, `x-api-key`, or `api-key` values into
those headers. This response risk is source verified but was not dynamically reproduced with a
secret canary. Mounted routes do not prove that v1.17.18 has durable multi-node execution.

### Akra Decision

Keep official `codex app-server` as runtime authority. Do not build an OpenCode-style provider/tool
daemon. Instead:

- reconcile only fields for which official thread/turn APIs provide current authority after child
  or transport loss, and mark unrecoverable event detail unknown;
- retain typed snapshots and gap-aware projections across TUI, Admin, CLI, Telegram, and automation;
- distinguish transcript continuity, client-detach continuity, and process-crash continuity;
- never claim that a restarted Akra process kept a turn alive without official evidence.

## TUI And Interaction Design

### Strengths To Adopt Selectively

OpenCode's TUI is the strongest part of the product. Its command palette and leader-based which-key
surface make a large command vocabulary searchable and teachable. Session handling includes search,
pinning, rename/delete, quick slots, model/provider/agent selection, fork, revert, compaction,
timeline, sharing, and child-session navigation. Permission and question flows preserve more typed
choices than Akra's current accept/decline modal. Diff and workspace views provide cohesive
drilldown without turning every fact into permanent chrome.

The terminal lifecycle also contains a pattern worth borrowing. Standard mode targets an internal
renderer loop and alternate screen. `--mini` uses a bounded split footer, native scrollback, replay,
and reset logic. Akra should keep its own inline-scrollback contract, but use the resize/replay
lessons to strengthen recovery rather than adopting OpenCode's viewport wholesale.

OpenCode hydrates only the latest 100 messages into TUI state, and its timeline reads that in-memory
set. The browser application pages older messages in batches. This is a sensible responsiveness
tradeoff, but a long-session timeline in the TUI is not complete history. Akra should use adaptive
bounded projections while preserving explicit truncation and an official load-more path.

### Akra Baseline

Akra already has a single `:` command registry, a searchable inline palette, model and session
overlays, native host scrollback, an official session browser, planning authority, and a parallel
operations board. The gap is not "add a command palette." The gap is to make the existing registry
contextual:

- expose whether an action is currently available and why;
- project the same metadata into palette, help, and optional leader/which-key discovery;
- preserve one execution definition rather than maintaining shortcut-only commands;
- keep approval dialogs modal and non-bypassable;
- collapse by task relevance and terminal width, not by a new generic widget framework.

## Sessions, Forks, And Background Work

OpenCode can fork a session by copying messages into a new session. The legacy fork does not retain
an explicit source/fork lineage on the new session. Akra's pinned official app-server schema already
contains `thread/fork`, optional turn selection, and `forkedFromId`, but Akra has no application path
for it. This is an opportunity to offer a narrower and more accountable workflow than transcript
copying: fork through the official protocol, preserve the returned lineage, and prove that the
original thread remains unchanged.

OpenCode also supports child sessions and experimental background subagents. The visible lineage
and navigation are useful UX. The background registry explicitly states that it is process-local
and non-durable; restart or owner-scope closure loses status and interrupts live work. Task
subagents share the current directory unless the operator separately creates a workspace. A
background completion is asynchronously injected into its parent as a synthetic prompt, while the
legacy runner joins an already-running loop instead of scheduling new work independently. It is
reasonable to infer a narrow race in which late injected work remains pending after the loop's last
history read; the audit did not reproduce it. Akra should borrow lineage and completion projection,
not same-checkout or process-local background authority.

## Web, Desktop, And IDE Surfaces

The shared application is broad and cohesive. It manages servers, projects, worktrees, sessions,
files, diffs, PTYs, providers, and settings. The CLI embeds its static build, while the hosted app
can connect to a server. Electron packages the same renderer with an embedded server and has useful
local/WSL integration. Renderer security settings include context isolation, disabled Node
integration, sandboxing, protocol path checks, and permission allowlisting. Desktop packaging also
configures hardened runtime/notarization and platform signing across six target combinations.

The cost is substantial surface and footprint. The v1.17.18 desktop installers observed through the
release API were roughly 104 to 158 MiB, and the product still labels desktop beta. A local build
produced a 2.8 MB web entry JavaScript chunk and a 6.7 MB Electron renderer entry before compression;
these are build-footprint facts, not startup or RAM measurements.

Browser server credentials, including username and password, are persisted through local storage.
Akra should reject that ownership model. Its Admin should remain an authenticated loopback operator
projection with server-side application authority, not become a generic remote IDE.

The VS Code extension is a pragmatic terminal launcher and context bridge. It starts `opencode` in a
split terminal, polls briefly for readiness, and sends the current file or selection to the TUI. It
is not an editor-native diff, chat, or code-action experience, and no extension tests were found.
Akra should not prioritize an IDE extension until its native operator loop is measurably superior.

## Worktrees, GitHub, And Delivery

OpenCode has a genuine worktree service. It can create `opencode/<slug>` branches and worktrees,
run startup commands, reset them, and remove them. Its reset path is deliberately destructive for a
non-primary worktree: hard reset, `clean -ffdx`, and submodule cleanup. This is useful interactive
workspace management, not task-owned delivery authority.

The GitHub action accepts several event types, starts an agent session, stages changes with
`git add .`, commits, pushes, and creates or updates a pull request. It checks actor repository
permission for user-triggered events. A generated workflow references the mutable
`anomalyco/opencode/github@latest`. If the agent changes branches, infrastructure delivery can be
skipped because the handler assumes the agent managed delivery itself.

For a public repository, the GitHub Action defaults to sharing the session unless `share: false` is
set. That occurs before event-specific delivery and uploads the session's messages, parts, diffs,
and model data through the share path. Hosted access, retention, deletion, and redaction policy were
not verified. This is a material trust tradeoff, not merely a collaboration affordance.

The audit did not find structural gates for required review, required checks, exact source-SHA
validation, serialized integration into a protected base, remote integration verification, or
conditional cleanup. OpenCode therefore demonstrates useful intake ergonomics, not a substitute for
Akra's delivery contract.

Akra's current reviewed default is materially stronger: one leased worktree per lane, frozen source
and target identities, a reviewed PR head, serialized integration worktree, remote verification,
and conditional branch/slot cleanup. Explicit parent-level high-risk autonomous modes remain
exceptions and must be projected as review-skipped, not counted as reviewed delivery.

## Safety And Trust Boundaries

### Reproduced Secret Disclosure Boundary

With `OPENCODE_SERVER_PASSWORD` set, an isolated `serve` process returned `401` without Basic Auth
and `200` with it. Without that variable, the same server only warned and allowed requests. A
canary configuration using `{env:...}` substitutions then produced this verified result:

- unauthenticated `GET /config` returned HTTP `200` and the resolved provider and MCP canaries;
- unauthenticated `GET /provider` returned the provider canary;
- the process was loopback-only for the experiment.

The source explains the result: variable substitution happens before parsing, the config handler
returns the resolved configuration, provider public mapping does not remove the legacy `key`, and
the authorization middleware is a pass-through when no password exists. Loopback reduces remote
reach but does not protect against other same-user local processes. Explicit host or mDNS settings
can widen the boundary. The server exposes HTTP rather than built-in TLS, and authentication also
accepts a URL query token. A non-loopback deployment therefore needs a separately operated TLS
boundary; Basic Auth alone is authentication, not transport confidentiality, and query credentials
can enter URL history or proxy logs.

### Capability Inconsistency

The default permission rules ask before direct reads of `.env` patterns. The shell tool separately
parses commands and asks for external-directory access outside the workspace, then checks its bash
pattern. It does not apply the direct-read permission to an in-workspace `cat .env`. Because the
default wildcard allows bash, source composition supports the inference that this shell path bypasses
the direct-read prompt. No authenticated model-driven exploitation was performed.

Akra should not react by implementing a second tool sandbox in front of Codex. It should verify the
official Codex sandbox and approval contract with planted canaries, define approved and forbidden
sinks for each credential/content class, and fail closed where Akra owns environment, approval,
logging, delivery, or transport. This can prove bounded non-amplification, not general data-loss
prevention or suppression of content the operator explicitly authorized Codex to read.

### GitHub Credential Lifetime

OpenCode writes a GitHub app token into repository-local `.git/config` as an HTTP extra header before
agent execution. If no previous header existed, its restore function returns without removing the
new value. The token is revoked later, but the active value is readable from the worktree during the
run and residue can remain afterward. Token exchange, attachment acquisition, and Git credential
configuration also precede the handler's actor-authorization check. The audit did not execute token
exfiltration.

Akra's GitHub writes should continue to authorize identity before credential mint or untrusted
download, then use isolated ephemeral credential contexts. A bounded egress contract must prove
that tokens reach the approved outbound authentication sink but do not enter prompts, source Git
configuration, logs, Admin responses, PR bodies, or durable application state.

### Plugins, MCP, And Supply Chain

OpenCode's extensibility is a genuine strength. Server plugins can alter provider/auth behavior,
chat parameters and headers, tools, permissions, shell environments, tool execution, and compaction;
TUI plugins can extend terminal behavior. That breadth lets advanced users adapt one harness rather
than wait for core releases.

It is also a trust boundary. Plugins run as trusted code in the server process and receive an
authenticated client, project/worktree paths, hooks, and `Bun.$`. Local MCP processes inherit the
full environment. Some LSP installers use mutable upstream artifacts such as a default branch
archive or `@latest` tool version. These are operator-trusted extension boundaries, not sandboxes.

Legacy provider auth and MCP OAuth state are stored as mode-`0600` plaintext JSON, while the v2
credential table stores a JSON value in SQLite. File modes reduce access by other OS users but are
not encryption or an OS keychain. Akra should not store raw third-party credentials in application
persistence; a future persistent secret dependency must use a separately owned secret-store handle.

Reject arbitrary in-process plugins as an Akra core direction. Ports should represent real outbound
boundaries; operator-selected MCP behavior remains under official Codex capability and approval
semantics.

## Performance

No comparative winner is established.

OpenCode has valuable performance instrumentation for the shared browser renderer: cold and warm
tabs, streaming throughput, animation-frame gaps, long tasks, layout mutation, and Chrome traces.
The suite runs against a production build with one worker. Its own documentation says results are
machine-dependent, does not assert portable budgets, and leaves packaged Electron process behavior
for future work.

The audit recorded artifact and build footprint, not interactive latency:

- Linux x64 CLI archive: 69,427,073 bytes;
- extracted Linux x64 binary: 188,979,328 bytes;
- web entry JavaScript: 2,815.62 kB before gzip, 842.34 kB gzip;
- Electron renderer entry: 6,731.22 kB before compression;
- Electron installers: approximately 103.9 to 158.3 MiB across primary targets.

Akra also lacks a complete, versioned process-tree benchmark. The existing
[Native Performance Evidence Contract](../jcode/gap-matrix.md#p0-native-performance-evidence-contract)
remains the correct dependency. OpenCode's renderer metrics can inform additional stress cases, but
must not create a second evidence schema or a claim based on incompatible numbers.

## Quality And Release Discipline

The exact source passed `bun typecheck`. The `packages/opencode` suite passed 3,110 tests with 22
skips, one todo, and zero failures. The app package script passed 567 unit tests plus 27 browser
tests under CI conditions, and the separately run, workflow-ungated desktop suite passed 61 tests.
The repository has substantial Linux/Windows unit and Chromium E2E job breadth, generated-client
checks, exact dependency installation, dependency-age policy, and SHA-pinned GitHub Actions.

There are meaningful gaps:

- an Arabic locale lacks three English keys; the focused parity test reproduced the failure, but
  that suite is skipped when `CI` is set;
- desktop test files exist but the desktop package has no `test` script, so the monorepo test task
  does not run them;
- root lint is not wired into the inspected GitHub workflows;
- the publish DAG does not structurally depend on test or typecheck jobs;
- the VS Code extension has a test configuration but no corresponding tests;
- the Nix desktop expression uses Electron 41 while the package manifest uses 42.3.3, and the Nix
  desktop evaluation is warning-only;
- external branch protection or release policy that might add gates was not verified.

These findings do not imply that the release is generally broken. They show why configured jobs and
code presence are weaker evidence than a release DAG bound to executed results.

## Akra Comparison

| Dimension | OpenCode v1.17.18 | Akra `354f4782` | Verdict |
| --- | --- | --- | --- |
| Product thesis | broad provider and surface-owning agent platform | official Codex operating and reviewed-delivery layer | deliberately different; Akra must state its wedge more clearly |
| Runtime authority | owns models, tools, provider auth, sessions, server, and clients | delegates model/tool execution to official app-server | preserve Akra's narrower boundary |
| TUI breadth | rich palette, key discovery, sessions, fork, timeline, diff, questions, permissions | inline host scrollback, searchable `:` registry, planning and parallel operations; protocol detail gaps | OpenCode ahead in interaction breadth |
| Detach continuity | separate `serve` plus `attach` can decouple client lifetime; default TUI owns and stops its worker; live detach not reproduced | child connection owns runtime; official transcript can resume | OpenCode has a stronger attach topology; end-to-end result remains inferred |
| External reconnect | retry without replay cursor or TUI full resync | restart recovery incomplete | both need explicit reconciliation; no durability winner |
| Fork lineage | transcript-copy fork without explicit source lineage | official schema supports `thread/fork` and `forkedFromId`; no Akra path | opportunity for Akra to differentiate |
| Background agents | child sessions; experimental process-local background jobs, shared directory | fixed worktree-isolated delivery lanes with durable lease/detail | OpenCode ahead in conversational fan-out; Akra ahead in isolation/delivery |
| Worktrees | interactive create/reset/remove and startup commands | leased lane lifecycle and identity-checked cleanup | Akra ahead for delivery; different interactive scope |
| GitHub automation | broad events and prompt-driven commit/push/PR | frozen source, review/check default gates, serialized integration, remote verification, cleanup | Akra ahead in delivery authority; OpenCode ahead in intake breadth |
| Server safety | password optional; raw resolved config/provider responses reproduced | loopback-only Admin, mandatory capability/session auth, origin guard | Akra structurally ahead; bounded egress proof still required |
| Approval fidelity | typed permission and question UI | bounded one-shot accept/decline; file approval and MCP elicitation decline | OpenCode ahead in choice fidelity |
| Web/desktop | shared full application and six desktop targets | authenticated operational Admin, no desktop | OpenCode ahead in product breadth; duplication rejected |
| IDE | terminal launcher and context bridge | none | OpenCode ahead, but low-priority for Akra |
| Performance proof | manual renderer suite without portable budgets | existing terminal validation, no versioned full process-tree benchmark | winner unknown; Akra evidence gap remains P0 |
| Quality | green Turbo core/app tasks plus separately run workflow-ungated desktop tests; release-gate gaps | broad Rust/native gate and delivery tests; Akra full CI was not rerun for this docs audit | no total quality winner |

## Decisions

### Adopt

- separate-server/attach lessons as evidence for the existing active-turn exit and restart-
  reconciliation contract, without claiming default-TUI detach or crash survival;
- typed permission/question presentation and exact decision preservation;
- official fork lineage, not transcript copying;
- richer diff, timeline, child, and long-session projection through the existing live execution and
  session provenance work;
- availability and reason metadata in Akra's existing command registry, plus optional leader-style
  discovery;
- GitHub event and schedule ergonomics only through the existing durable automation intake plan;
- renderer stress dimensions as additions to the existing performance evidence contract.

### Reject

- a multi-provider or local tool runtime beside official Codex;
- arbitrary same-process plugins and an Akra-owned LSP installer;
- optional authentication for any control or secret-bearing API;
- raw configuration, provider, credential, filesystem, PTY, or shell mutation over Admin;
- browser-stored server credentials, public-repository default sharing, automatic sharing, and a
  full browser IDE;
- same-checkout background agents and destructive generic worktree reset;
- prompt-only delivery, `git add .`, branch-management bypass, or floating `@latest` automation;
- a desktop or IDE expansion before native TUI performance and the delivery loop are measured.

### Differentiate

- direct official Codex protocol fidelity with typed approval, elicitation, steering, fork, diff, and
  thread provenance;
- authenticated loopback Admin plus a source-to-sink known-canary release matrix;
- exact-source, worktree-isolated, reviewed delivery with explicit high-risk exception provenance;
- durable planning and automation lineage from trigger through session, branch, PR, integration, and
  cleanup;
- a truthful game-operations projection driven only by durable lifecycle and activity transitions;
- native performance claims only after repeatable full-process-tree evidence exists.

## Refresh Triggers

Refresh this analysis when:

- OpenCode ships a new major runtime or makes v2 server/session ownership the default;
- server authentication, public DTO redaction, GitHub credentials, SSE replay, background durability,
  or permission composition changes;
- a release materially changes TUI long-session hydration, fork lineage, worktree delivery, desktop
  packaging, or the performance suite;
- Akra implements typed approvals/elicitation, official fork lineage, reconnect reconciliation, or a
  known-canary bounded egress matrix;
- a fair authenticated OpenCode/Akra interactive benchmark becomes available.

## Sources

See [evidence.md](evidence.md) for immutable links, commands, raw observed results, and explicit
limits. See [gap-matrix.md](gap-matrix.md) for dependency-aware implementation contracts.
