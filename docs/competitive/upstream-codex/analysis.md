# Upstream OpenAI Codex v0.144.1 Baseline Audit

[한국어 번역](../../ko/competitive/upstream-codex/analysis.md)

This audit compares the upstream OpenAI Codex release `rust-v0.144.1` with Akra at prerelease
commit `226e4794b84107704378ecc1ea65f7d5c27750e5`. Upstream Codex is not a normal competitor: its
`codex app-server` is Akra's runtime authority, while its first-party TUI is the fastest-moving
reference client for that authority. Immutable source links, artifact hashes, reproduced commands,
raw measurements, and limitations are in [evidence.md](evidence.md). Akra-relative decisions and
implementation contracts are in [gap-matrix.md](gap-matrix.md).

## Executive Verdict

Upstream Codex v0.144.1 is already a broad native coding environment, not merely a CLI behind a
JSON process boundary. Its default TUI runs app-server in process over bounded typed channels,
while explicit remote and local-daemon paths preserve the same thread, turn, item, approval, and
session semantics. The official runtime owns model execution, tools, sandboxing, approvals, MCP,
skills, plugins, hooks, account/config state, compaction, sessions, fork, goals, subagents, remote
control, and generic command/filesystem/process APIs.

The strongest threat to Akra is structural: an official client can remove the child-process and
external JSON boundary while exposing protocol changes on the release that introduced them. Akra
currently pays the separate app-server process cost yet loses material protocol truth, including
actual model/runtime configuration, most item kinds, command and patch progress, token usage, and
failed versus interrupted terminal state. The most urgent finding is not a missing feature but a
truth bug: Akra translates every matching `turn/completed` notification into success and treats
every `error` notification as terminal, even though the official protocol carries typed terminal
status and explicitly marks retrying errors as nonterminal.

Akra should not respond by cloning Codex tools, providers, session storage, remote control, or the
experimental daemon. It should become the most faithful reviewed operating layer on top of the
official runtime, then differentiate where upstream does not claim authority: accepted planning,
isolated worktree leases, frozen-source validation, pull-request review, integration, remote
verification, and cleanup. The immediate order is terminal truth, a closed typed live projection,
steering and recovery, official model capability projection, then shared TUI/Admin/parallel views.
Only after those foundations are correct should Akra deepen session search, fork, approval forms,
or additional input modalities.

No performance winner is established. A ten-run unauthenticated direct app-server probe is useful
as a lower-bound baseline, not as a comparison with either interactive TUI. The official TUI's
in-process topology is a verified architectural advantage over Akra's current transport path, but
its end-user latency and memory effect require a controlled same-machine interactive benchmark.

## Snapshot

| Field | Value |
| --- | --- |
| Product | OpenAI Codex |
| Official repository | <https://github.com/openai/codex> |
| Release | [`rust-v0.144.1`](https://github.com/openai/codex/releases/tag/rust-v0.144.1) |
| Peeled source commit | `44918ea10c0f99151c6710411b4322c2f5c96bea` |
| Release published | 2026-07-09 23:02:40 UTC |
| Audit date | 2026-07-12 (Asia/Seoul) |
| Akra baseline | `226e4794b84107704378ecc1ea65f7d5c27750e5` on `prerelease` |
| Akra version | 1.3.5 |
| Auditor environment | Ubuntu 24.04.2 WSL2, Linux 6.18.33.2, x86_64 |
| Toolchain | Rust/Cargo 1.95.0, Node 24.14.1, npm 11.6.1 |

The release source was checked out detached at the peeled tag commit. Official x86_64 Linux musl
CLI and app-server archives were downloaded and matched the release checksums. Cargo test setup
updated the temporary source checkout's lockfile version fields, so source conclusions and links
use the immutable commit rather than the later local working-tree state. No Akra source file was
modified during evidence collection.

## Product And Audience

The official product serves operators who want the newest Codex model and tool behavior through a
terminal-first interface, noninteractive execution, IDE/app-server integrations, or remote-control
experiments. The installed `codex` binary exposes interactive TUI, `exec`, `review`, session
resume/archive/delete/unarchive/fork, MCP, plugins, app-server, cloud, remote control, sandbox,
doctor, and feature inspection. That breadth means Akra cannot differentiate by exposing an
app-server session, a model picker, or a generic agent transcript alone.

The official product optimizes the coding interaction itself. It persists Codex rollouts and owns
runtime reconstruction, but it does not provide Akra's repository policy that every meaningful
slice receives a dedicated worktree, commit, pushed branch, pull request, critical review response,
rebase integration, remote verification, and cleanup. The distinction is authority, not a claim
that upstream cannot invoke Git or GitHub tools in a turn.

Akra's target operator is narrower: someone who wants official Codex behavior embedded in a durable
delivery system with inspectable intent and cross-surface operational state. Product work should
therefore project official capability faithfully and spend Akra-owned complexity on delivery
truth rather than provider or tool breadth.

## Runtime Architecture

### Runtime authority

Codex core is the authority for configuration, prompts, model/provider requests, tool execution,
sandbox and approval policy, MCP/apps/plugins, compaction, rollout persistence, thread lifecycle,
and subagents. App-server maps that authority into versioned JSON-RPC concepts: thread, turn, item,
server request, notification, model, permission profile, goal, hook, and remote-control state.

Akra correctly delegates model and tool execution to that boundary, but its adapter is not yet a
lossless application-facing projection. A passing method-vocabulary classification test proves
that method names were noticed; it does not prove that payload meaning survives the adapter,
reducer, persistence, and inbound surfaces.

### Three first-party TUI paths

The first-party TUI supports three runtime shapes:

1. An embedded in-process app-server used by default.
2. A local app-server daemon reached over a Unix-domain socket when its launch configuration is
   compatible and replayable.
3. An explicit remote app-server endpoint over Unix socket or WebSocket.

The embedded path uses typed `ClientRequest`, `ClientNotification`, and `InProcessServerEvent`
channels. JSON-RPC result envelopes remain, but serialization and the process boundary are removed
from the hot path. Queues are bounded and overload or lag becomes explicit. This is a credible
design response to latency and memory pressure without introducing a second semantic protocol.

The local daemon path is not a production contract Akra should adopt now. Its README marks it
experimental, it is Unix-only, its updater is not reboot-persistent, and auto-update may restart an
active app-server. The remote client surfaces disconnect as an event and does not contain a general
reconnect loop. These limitations make daemon/UDS a bounded recovery experiment, not a basis for
promising client-independent continuation.

### External app-server boundary

The released app-server listens through stdio, Unix sockets, or plain `ws`; its remote client also
accepts `wss`. Capability-token or signed-bearer authentication modes protect WebSocket. The binary
rejected an unauthenticated non-loopback bind in the audit environment, and source rejects WebSocket
upgrade requests carrying an Origin header. Plain `ws` is intended for loopback or an SSH-forwarded
path; nonlocal use needs a TLS reverse proxy plus authentication.

That transport protection does not make raw app-server a safe Akra Admin API. Stable methods include
host filesystem and command operations, and an initialized isolated probe read `/etc/hostname`
through `fs/readFile`. Stable filesystem and thread shell-command paths can act with local-user
authority; experimental process spawn is unsandboxed and inherits app-server environment. Akra
should continue exposing bounded authenticated application DTOs rather than forwarding the generic
JSON-RPC surface through Admin or Telegram.

## Protocol And Capability Surface

The generated v0.144.1 stable schema contains 87 client requests, 68 server notifications, 10
server requests, and one client notification. Enabling schema export with `--experimental` expands
client requests to 122 and server requests to 11 while notification counts remain unchanged. The
runtime capability handshake independently gates experimental use; generating or vendoring an
experimental schema does not enable those methods.

Akra's pinned schema was generated from 0.144.0 with experimental entries. After normalizing a
fresh 0.144.1 experimental export using Akra's own normalizer, the definitions were byte-identical;
only generated date and source CLI metadata differed. This verifies patch-level vocabulary currency,
not semantic coverage and not a stable-runtime guarantee. Akra initializes app-server with
`experimentalApi: false`, so product contracts must distinguish stable methods from merely present
experimental definitions.

An isolated released-binary handshake successfully read account, thread catalog, loaded threads,
models, collaboration modes, apps, skills, plugins, remote-control status, permission profiles,
hooks, config, MCP status, and experimental-feature catalog. Negative probes verified the required
initialization order, one-time initialization, capability gating, and notification opt-out. These
are direct runtime observations without model authentication or a live turn.

Stable discovery is not stable application for every returned capability. In particular,
`permissionProfile/list` is stable, but named/custom `thread/start.permissions` selection and
`activePermissionProfile` response provenance are experimental. The stable thread path retains
legacy `sandbox` and a generic config override map, so an existing custom default or untyped
`config.default_permissions` override can still affect execution without returning profile identity.
Akra's normal stable product path must not use that generic escape hatch for permission selection;
it may map only exactly representable built-in profiles to legacy `sandbox` and must treat arbitrary
named profile selection as unavailable unless a separate experimental product decision is made.

### Terminal state is richer than Akra's projection

Official `Turn` data carries `completed`, `interrupted`, `failed`, or `inProgress` status, optional
typed error, timestamps, duration, and item-view completeness. The `error` notification nests a
`TurnError` and declares `willRetry`; `willRetry: true` explicitly does not interrupt the turn.

At the audited Akra baseline:

- the `error` handler looks for nonexistent top-level `params.message` and terminates the stream;
- `turn/completed` validates IDs but discards `turn.status` and `turn.error`;
- the terminal event send result is discarded, and connection/callers reduce completion to
  transport-level `Result<()>` even when the application sink disconnected;
- the core reducer receives only a generic `TurnCompleted` event;
- a parallel worker archives its thread whenever the adapter returned `Ok`;
- prompt-log status records `completed` whenever the stream returned `Ok`.

An interrupted turn reaches this path as successful Akra completion. A failed
`turn/completed` payload is likewise indistinguishable from success if it reaches the handler
without an earlier nonretrying `error` having already aborted the loop. Separately, a transient
upstream error terminates Akra even while upstream is retrying. Until fixed, completion-dependent
planning continuation, archive, validation, and delivery state must not treat the current adapter's
generic success as authoritative proof.

### Method coverage is not item coverage

Akra classifies all current notification method names into handled, deferred, diagnostic, or
ignored sets. However, the active reducer drops every deferred method into a warning because no
adapter translation exists. Live completed items preserve only agent messages, file changes, and
command executions; historical snapshots add user messages. The official `ThreadItem` vocabulary
also carries reasoning, plan, MCP calls, dynamic tools, collaboration and subagent activity, web
search, image generation, review-mode transitions, compaction, and other typed outcomes.

The correct response is a closed application projection with an explicit unknown variant and
bounded payloads, not mirroring every wire struct into `domain`. Started/completed identity,
terminal truth, command output, patch and turn diff, plan, token usage, reroute, approval ownership,
and parallel activity are operationally meaningful. Fine-grained reasoning text, raw secrets, and
unbounded tool output require deliberate redaction and size policy.

## Models, Tools, Context, And Sessions

### Model and applied runtime envelope

Official `model/list` supplies model IDs, display labels, default selection, supported reasoning
efforts, input modalities, service tiers, and related capabilities. The effort vocabulary is owned
and versioned upstream: its current enum already includes `max` and `ultra` beyond Akra's hard-coded
picker, and a future release can add another wire value. Thread start and resume responses also return the actual
applied model, provider, cwd, approval policy, sandbox, reasoning effort, and service tier, while
`model/rerouted` can change effective model truth later.

Akra currently hard-codes model/effort choices and deserializes only the `thread` portion of start
and resume responses. It should request and project the official catalog, preserve unknown effort
values, and show requested versus applied configuration. It should not add provider routing or an
Akra-owned model registry.

### Tools and extensibility

Codex owns shell/command, patch, filesystem, web, image, dynamic tools, MCP, apps, plugins, skills,
hooks, and sandbox behavior. Feature discovery in the released binary returned 92 rows spanning
stable, experimental, under-development, and removed compatibility entries. A feature being listed
or enabled is not the same as a stable app-server method or a mature product workflow.

Akra should expose capability/status discovery only where it supports an operator decision. For
example, model availability, MCP startup failure, plugin/skill availability, permission profile,
and effective sandbox belong in diagnostics or the live envelope. Generic filesystem/process/plugin
management remains upstream authority.

### Session reconstruction and compaction

Upstream rollout persistence, resume reconstruction, rollback, fork, and compaction are substantial.
The reducer reconstructs replacement-history checkpoints, world state, settings, and compaction
windows. Fork explicitly handles a mid-turn snapshot boundary by inserting an aborted-turn marker.
Manual and automatic compaction have provider-specific paths and persist replacement history.

The audited source also contains a pre-turn compaction limitation: it checks existing context usage
without estimating the new input and context diff, so a large input can cross the threshold before
the first sampling attempt. This is an upstream runtime risk to surface through error and recovery
truth, not a reason for Akra to build its own compactor.

Akra's official session catalog projection loses cursor-driven paging, several filters, lineage,
and provenance fields. That gap belongs to the existing Official Session Search And Provenance
slice. Experimental turn/item paging must not be described as stable while Akra initializes without
experimental capability.

### Memory

Automatic memory is experimental and off by default. Its extraction path removes selected prompt
fragments and applies best-effort secret sanitization, while its isolated consolidation worker has
strong sandbox restrictions. The sanitizer is intentionally pattern-based rather than complete,
and the memory rate-limit guard allows work to proceed when its check fails. Those facts argue
against adding a second Akra semantic-memory store. Accepted planning and explicit provenance are
the safer near-term memory primitives.

## TUI And Interaction Design

The first-party TUI is the reference for breadth and protocol freshness. It supports session
resume/fork, goals, steering, subagents, model/reasoning selection, permission profiles, skills,
plugins/apps, hooks, review, compaction, command discovery, rich approvals, and remote operation.
Approval UI preserves thread/environment context, supports cross-thread navigation, and separates
one-shot, session, persistent-rule, host, decline, and abort semantics when offered.

There are still reference-client limitations worth treating as upstream facts rather than Akra
targets. Approval requests added while a modal is already visible are stored in a vector and later
popped, so sustained arrivals can favor newer requests. The normal exec decision list may omit the
protocol's decline-and-continue choice even though the TUI can render it. A closed Akra approval
queue should therefore preserve arrival order and render only the authoritative offered decisions,
not copy first-party defaults blindly.

Akra remains ahead in a deliberately different terminal contract: inline rendering preserves host
scrollback instead of owning a full-screen transcript history. It also has planning and parallel
delivery overlays tied to application state. Those advantages are weakened when live Codex items
collapse into one generic activity line. The right design is a bounded priority rail and drilldown
over typed application events, while keeping host scrollback as the durable transcript surface.

## Admin And Remote Surfaces

Upstream app-server, WebSocket remote TUI, and `remote-control` cover generic Codex access. The
remote-control and daemon surfaces are explicitly experimental. The local remote-control database
stores server/environment identity, not the bearer token. No audited upstream surface provides
Akra's planning authority, lease board, validation evidence, GitHub review/integration state, or
delivery game diorama.

Akra should keep Admin as an authenticated projection of the same application services used by the
TUI, CLI, Telegram, and automation. It must not become a raw app-server proxy. A later Admin status
view may ingest bounded, redacted facts from `codex doctor --json`, but only after the output schema,
command cost, timeout, redaction, and offline behavior are pinned. Doctor performed reachability
checks during this audit, so it is not a harmless synchronous render helper.

## Parallel Work

Stable first-generation Codex subagents are enabled by default, bounded by a default concurrency of
six and depth one, and inherit live model/provider/reasoning, approval, cwd, permission profile,
environment, and compatible execution policy. This makes upstream subagent events part of the
normal runtime vocabulary that Akra should project.

The second-generation graph/fanout implementation is under development and off by default. Source
inspection found missing depth enforcement in its tool gate and a possible check-then-increment
race in its execution limiter. These are conditional source inferences, not reproduced load
failures, and should not be scored as shipped defects or copied into an Akra roadmap.

Codex subagents coordinate model work inside the runtime. They do not replace Akra's accepted task
authority, one-worktree-per-lane lease, frozen source, validation, PR review, integration, and
cleanup. Akra should map official subagent and collaboration items into current activity while
keeping delivery state authoritative in its own services.

The TUI's `/side` lifecycle interrupts and unsubscribes from an ephemeral thread. App-server can
retain an unsubscribed inactive thread for a per-thread 30-minute no-subscriber-and-inactive delay,
so repeated side sessions can temporarily accumulate resources. This is a bounded transient-risk
inference, not lifetime leak evidence.

## Performance

### Reproduced direct app-server lower bound

The released x86_64 Linux musl app-server was measured in a fresh isolated `CODEX_HOME`, without
authentication or a model turn. Each of ten warm-cache runs launched the default app-server,
initialized, called `account/read`, called `thread/list`, and exited. Elapsed samples in
milliseconds were:

```text
120.781 122.712 123.812 120.261 117.292
120.708 122.124 120.605 119.439 120.829
```

Minimum was 117.292 ms, median 120.745 ms, and maximum 123.812 ms. Sampled peak process-tree RSS
values in KiB were:

```text
82440 90524 93328 92052 87676 88272 88352 88336 91224 87920
```

Minimum was 82,440 KiB, median 88,344 KiB, and maximum 93,328 KiB; every sample peaked at four
processes. Manual inspection of the same default path observed app-server's enabled plugin startup
warmup running `git ls-remote https://github.com/openai/plugins.git HEAD` plus Git transport helpers.
The final response does not wait for that public-network work to finish. The checked-in harness
requested 2 ms all-thread recursive `/proc` process-tree polling, but event-loop scheduling and RSS
sampling can miss a shorter peak. Page cache was warm, the path had no TTY/rendering, authentication,
model call, stream, MCP request, or Akra process. These numbers describe a default-path lower bound,
not isolated protocol cost or a user-visible winner.

### Structural and memory risks

The official embedded TUI avoids a child process and external JSON serialization, while Akra's
current TUI launches app-server. This makes full process-tree accounting mandatory and creates a
credible optimization target, but only a controlled PTY benchmark can quantify it.

Within Codex core, submission is bounded but one event channel before the bounded app-server
downstream is unbounded. A slow client or delta-heavy multi-agent run can therefore accumulate
events in core memory. This source inference was not stress-reproduced. Akra should benchmark event
backlog and full-tree RSS under fixed synthetic streams, bound its own queues, and preserve upstream
lag/overload state rather than assuming the official runtime makes memory unboundedness impossible.

## Safety And Quality

### Sandbox and transport

The implicit sandbox is trust-state-dependent: either recorded project decision (`Trusted` or
`Untrusted`) selects the built-in workspace profile, while no trust decision selects read-only;
both retain root read and restrict network. Explicit CLI policy can change this. Product copy must
show the effective applied envelope rather than claim one universal default.

The upstream generated-shell default is broader than Akra's launch policy: it inherits all parent
environment variables and disables the default key/secret/token exclusion rules. Akra already
clears and allowlists the app-server child environment, sets generated-shell inheritance to `core`,
and preserves secret excludes unless an operator chooses an explicit elevated override. That is a
real structural advantage, but it needs a released-binary canary because constructed command
arguments alone do not prove the eventual tool-child environment or login-profile behavior.

Filesystem policy points the other way only before a project decision exists. The first-party
runtime selects read-only with no trust decision and the workspace profile after either a Trusted or
Untrusted decision, while Akra's normal main and parallel work turns default to workspace-write
without an equivalent recorded decision. Akra thread setup and hidden planning paths remain
read-only, so this is not a universal runtime default. Akra must not combine its stronger
environment boundary with a blanket claim of safer defaults; requested and applied sandbox/profile
plus the operator decision must remain explicit.

WebSocket non-loopback and Origin protections are meaningful strengths. They do not narrow the
post-initialization method authority. Akra's remote surfaces should remain capability-specific,
authenticated, rate-limited, audited, and unable to submit arbitrary app-server JSON-RPC.

Rollout files can contain messages, reasoning, tool calls, commands, and output. The recorder uses
normal create semantics and does not itself force mode `0600`, so protection depends on directory
permissions and umask. This is a conditional local confidentiality risk, not evidence that an
audited user's rollout was exposed. Akra should avoid duplicating raw transcript persistence and
must give its own prompt/review logs explicit file/database permissions, retention, redaction, and
Admin authorization.

Codex authentication also defaults to a plaintext `$CODEX_HOME/auth.json` file containing API,
OAuth, PAT, or agent-key material. Its Unix create path requests mode `0600`, but saving an existing
file does not repair looser permissions; keyring and encrypted alternatives exist. Akra does not own
that upstream store, and environment scrubbing does not protect a readable file under the same home.
Secret-canary work should observe direct/shell reads and fail closed on Akra's secondary
amplification surfaces without claiming it can police an authorized upstream read.

### Test and release evidence

The exact tag commit has a successful `rust-release` workflow with 44 attached checks, dominated by
build, signing, packaging, and publishing work. That is strong artifact provenance but not proof
that the complete normal pull-request test matrix reran on the tag commit. The repository contains
broad Cargo/Bazel workflows, but this audit keeps exact-tag release evidence separate.

The documented simple x86_64 Linux CLI and app-server tar archives each contain one binary. On the
audit host, which lacked system `bwrap`, both failed sandboxed execution with exit 101 because the
adjacent bundled helper was absent. Upstream also publishes a complete package archive that contains
and validates the helper. This is a shipped install-shape defect, not a universal Linux failure.
Akra packages neither Codex nor `bwrap`; its correct response is an exact install-compatibility
receipt and operator prerequisite, not silently vendoring upstream resources.

Release build/signing hardening is broad, but the final release job does not directly depend on the
normal source-test matrix and its Cargo release commands do not consistently use `--locked`. The
committed tag lockfile stores workspace packages as `0.0.0`, and a local Cargo invocation rewrote
132 entries to `0.144.1`; no external dependency graph change was observed. Akra already uses
locked builds. Its remaining differentiation is to bind source-test receipts, Codex compatibility,
and archive integrity independently to the release SHA before mutation.

After supplying user-space `pkg-config` and OpenSSL development files, the exact source passed all
256 `codex-app-server-protocol` tests and all 27 `codex-app-server-client` tests. The v0.144.1 patch
itself changed eight files concentrated in installer and code-mode behavior; app-server protocol
conclusions therefore also depend on the wider v0.144.0 release lineage, not on a claim that the
patch introduced them.

`codex doctor --json` produced structured redacted checks in an isolated home. Expected credential,
terminal, and install-match checks failed in that probe, while app-server, config, Git, state, and
sandbox checks ran. It also attempted reachability work. Treat doctor as a diagnosable command with
side effects and latency, not an always-safe data library.

## Akra Comparison

| Dimension | Upstream Codex v0.144.1 | Akra `226e4794` | Verdict | Consequence |
| --- | --- | --- | --- | --- |
| Runtime authority | official model/tool/sandbox/session authority | delegates execution to app-server | deliberately different | Keep delegation; never fork runtime semantics |
| Hot-path topology | default typed in-process app-server | child process plus external JSON-RPC | upstream structurally ahead | Measure full tree; test a bounded official-client/UDS viability path only |
| Protocol currency | release-native stable/experimental gating | 0.144.0 experimental schema, stable runtime handshake | vocabulary current, capability contract blurred | Pin stable and experimental artifacts separately |
| Terminal truth | typed completed/interrupted/failed/retrying | generic completed; retrying errors terminate | upstream decisively ahead | Fix before any completion-dependent automation |
| Live items | broad typed item and notification set | most payload meaning dropped | upstream ahead | Build closed typed projection with unknown/redaction |
| Model selection | official dynamic capabilities and applied envelope | hard-coded catalog and effort enum | upstream ahead | Add narrow official catalog port and effective-state projection |
| Sessions | persistence, paging, filters, lineage, fork, resume | basic list/read/resume projection | upstream ahead | Complete existing session provenance slice after P0 truth |
| TUI breadth | reference support for most runtime features | narrower Codex view, richer planning/delivery overlays | upstream ahead on Codex interaction | Preserve host scrollback; improve high-priority live detail |
| Admin/delivery | generic remote/experimental control | planning, review, metrics, Telegram, GitHub delivery | Akra ahead in owned workflow | Share authoritative application state, not raw RPC |
| Parallel runtime | bounded native subagents and developing graph mode | worktree-isolated lanes and delivery services | complementary | Project runtime activity; retain Akra lease/integration authority |
| Reviewed integration | Codex can use Git/GitHub but no equivalent product invariant | commit/PR/review/integration/cleanup default with explicit bypasses | Akra ahead in verified contract | Make evidence and bypass provenance visible |
| Performance evidence | direct lower bound plus in-process architecture; no fair TUI comparison | existing P0 benchmark contract, no current comparable artifact | winner unknown | Produce one shared protocol/PTY/full-tree benchmark |
| Remote safety | protected transports but broad post-init host methods | bounded application APIs and identity-gated GitHub writes | Akra has the safer intended boundary | Keep raw app-server private |
| Linux install readiness | simple tar fails sandbox without system `bwrap`; complete package carries helper | external Codex prerequisite; no install-shape receipt | upstream defect can break sandboxed command/runtime use | Narrow operator prerequisite now; add compatibility receipt; do not vendor helper |
| Shell environment | inherits all and disables secret excludes by default | clears/allowlists child and defaults tool shell to core | Akra structurally ahead | Extend released canary across every runtime path |
| Filesystem sandbox default | no trust decision is read-only; either recorded decision uses workspace profile | normal main/parallel work turns default workspace-write; setup/planning read-only | upstream safer for normal work before a decision exists | Add explicit trust/profile choice to existing permission P1 |

## Decisions

### Adopt

- Typed in-process semantics as the reference architecture: keep one app-server vocabulary and
  remove translation loss before considering transport optimization.
- Negotiated stable/experimental capability contracts with separate generated artifacts and
  negative tests.
- Official model capability and actual applied-envelope projection.
- Typed terminal, item lifecycle, token, patch, diff, plan, reroute, compaction, approval-clear,
  collaboration, and subagent facts through bounded application types.
- Session paging, filters, lineage, and fork through the existing session provenance work.
- Bounded queue, overload, lag, and shutdown semantics in every Akra-owned event path.
- First-party approval information hierarchy while preserving authoritative offered decisions and
  FIFO ownership.
- Exact Codex install-shape and sandbox-helper compatibility receipts, separate from Akra archive
  integrity and source-test evidence.

### Reject

- An Akra provider/model execution registry, tool engine, MCP runtime, compactor, or session store.
- Exposing raw app-server JSON-RPC through Admin, Telegram, or automation.
- Treating experimental local daemon or remote-control behavior as a production continuity promise.
- Vendoring upstream `bwrap`, credentials, or a Codex package manager into Akra in response to an
  operator-installed runtime defect.
- Turning Codex goals, plan items, review mode, or subagent completion into Akra delivery authority.
- A second automatic semantic-memory system before official session provenance and planning links.
- Copying feature flags or source presence into product claims without stage and handshake proof.

### Differentiate

Akra should be more truthful than the reference client at the delivery boundary:

- requested versus applied runtime configuration remains visible;
- retrying, interrupted, failed, unknown, and completed are distinct and durable;
- an upstream item completing does not mean a planning task or delivery completed;
- Codex `EnteredReviewMode` is not GitHub review evidence;
- accepted planning intent, worktree lease, source freeze, validation, review, checks, integration,
  remote verification, and cleanup remain separate authoritative states;
- TUI, Admin, CLI, Telegram, and automation consume the same application projection;
- every autonomous review/check/PR bypass remains explicit policy provenance.

## The Akra Wedge

The durable product statement after this audit is:

> Akra is the Codex-first operating and delivery layer that projects official runtime truth without
> distortion, then turns accepted intent into isolated, validated, reviewed, remotely verified
> integration.

The phrase "projects official runtime truth" is now a prerequisite, not positioning copy. Akra
cannot credibly claim a stronger delivery layer while it discards official failed/interrupted
status and labels a matching completion as success. Once that is corrected, upstream velocity
becomes leverage: model, tool, session, and subagent improvements arrive through the official
authority, while Akra concentrates on the operator and delivery guarantees that upstream does not
own.

## Refresh Triggers

Refresh this audit when any of the following occurs:

- a Codex release changes app-server stable methods, terminal status, applied thread envelope, TUI
  transport selection, daemon support, or remote authentication;
- the official TUI makes daemon/remote reconnect a supported production contract;
- Akra replaces its child-process adapter or opts into experimental app-server capability;
- the terminal-truth/live-projection P0 or official-model P1 lands;
- a controlled authenticated interactive TUI benchmark or event-backpressure stress test becomes
  available;
- upstream changes its simple/complete Linux archive layout, sandbox-helper discovery, shell
  environment default, or auth storage default;
- upstream memory or multi-agent V2 moves from experimental/development to stable default behavior.

## Sources

See [evidence.md](evidence.md) for the immutable source ledger, release artifact identity,
reproduction commands, direct app-server samples, test output, Akra baseline links, and audit
limits. See [gap-matrix.md](gap-matrix.md) for owned implementation deltas and proof requirements.
