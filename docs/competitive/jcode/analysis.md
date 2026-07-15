# jcode v0.43.0 Deep Dive

[한국어 번역](../../ko/competitive/jcode/analysis.md)

This analysis compares jcode v0.43.0 with Akra at prerelease commit
`66333152170124a42aca6f49ed2f72fa6a8293d7`. Source links, commands, counts, and limitations are in
[evidence.md](evidence.md). Product decisions and implementation slices are in
[gap-matrix.md](gap-matrix.md).

## Executive Verdict

jcode is the strongest direct warning that "native Rust Codex wrapper with a TUI" is no longer a
position. It owns a broad agent runtime and competes on claimed speed, a shared multi-session
architecture, high-density TUI instrumentation, memory, provider choice, and recursive swarm
coordination. It is not merely a polished chat screen around a model.

Its strongest advantages over Akra are:

1. performance is treated as public product value and backed by executable measurement scripts,
   even though the published numbers are not sufficient for a current, fair comparison;
2. the TUI exposes model, context, memory, git, workspace, diagrams, todos, background work, and
   swarm state through prioritized, space-aware instrumentation;
3. the daemon and protocol are designed to amortize shared state across sessions and make them easy
   to attach, reconnect, inspect, and coordinate, although this audit did not reproduce the cost;
4. its provider, tool, hook, memory, and swarm breadth gives power users many reasons to stay inside
   one harness.

Its exploitable weaknesses are equally structural:

- jcode owns provider, auth, tool, memory, daemon, TUI, desktop, swarm, and self-development stacks;
  that creates a very large security and maintenance surface;
- current and proposed behavior are often adjacent in the same product narrative;
- same-repository swarm coordination is powerful but does not by itself prove isolated delivery,
  review incorporation, or linear integration into a protected base;
- the audited release fails its checked-in code-size ratchet despite extensive quality tooling;
- published performance data uses an older jcode build, incomplete environment evidence, and no
  tracked raw result artifact.

Akra should not answer by cloning jcode's runtime breadth. It should answer with a narrower, sharper
product:

> the fastest and most trustworthy way to operate official Codex sessions from intent through
> reviewed integration by default.

That requires Akra to close its measurement and interaction gaps, then amplify what jcode does not
center: Codex protocol fidelity, operator-owned planning authority, worktree isolation, serialized
reviewed-range delivery, critical review handling, and shared operator truth across TUI and Admin.
Akra's explicit parent-level autonomous-delivery opt-in can bypass review/check gates and, in
eligible PR modes, PR automation itself. Those high-risk escape paths must remain visibly
review-skipped rather than being presented as fulfillment of the reviewed-delivery promise.

## What jcode Actually Is

jcode describes itself as a next-generation coding-agent harness for multi-session workflows,
customizability, and performance. The source supports that description. It is a 77-crate Rust
workspace with approximately 518k production Rust LOC under this audit's coarse counting rule.

The product owns:

- a long-lived server/daemon and local client protocol;
- multiple provider and subscription login paths;
- an agent and tool runtime;
- TUI presentation, terminal image, Markdown, Mermaid, permission, account, session, usage, and
  workspace crates;
- memory extraction, retrieval, graph types, embeddings, and session search;
- swarm coordination, messaging, plan graph, task state, and optional worktree behavior;
- background, ambient, overnight, telemetry, self-development, gateway, desktop, and mobile-facing
  directions.

This breadth is the product. Calling it a TUI client understates the competitive threat and also
explains its maintenance cost.

## Evolution Since v0.11.2

The prior internal jcode analysis was based on v0.11.2. Between that tag and v0.43.0:

- tracked Rust files grew from 735 to 1,084;
- Rust LOC grew from 336k to 632k;
- root plus workspace crates grew from 35 to 77;
- root `src` Rust files fell from 631 to 45 as code moved behind crate boundaries;
- TUI-family Rust LOC grew from about 120k to 213k;
- desktop-family Rust LOC grew from about 12k to 72k.

This is not superficial churn. jcode added dedicated TUI presentation crates, shared render models,
smoothness and anchor-stability work, expanded provider runtimes and diagnostics, a much larger
desktop prototype, and a DAG-oriented swarm direction.

It also reveals the competitive pace Akra must plan against. A one-time feature comparison becomes
stale quickly. The durable lesson is jcode's investment pattern: performance measurement, richer
operator visibility, and more autonomous parallelism receive continuous product work.

The modularity result is mixed. Extraction reduced the root crate, but a large serial dependency
spine remains through `jcode-tui`, `jcode-app-core`, and `jcode-base`. Scope growth moved behind more
boundaries without eliminating large ownership units. Akra should copy the measurement and focused
crate/module discipline, not the assumption that more crates make broad scope inexpensive.

## Runtime Architecture

### jcode

jcode uses one server process as session and runtime authority. TUI clients connect through a local
socket, normally attach to one session, and reconnect after server reload or transport loss. The
server owns provider state, tools, memory work, persistence, background work, and swarm state.

This architecture has important user-facing consequences:

- an additional client can reuse server-owned state instead of launching another complete runtime;
  lower incremental cost is a topology-based inference until reproduced;
- killing a client does not kill its session runtime;
- a server can coordinate sessions and file-read/file-change awareness centrally;
- self-development can replace the server binary and reconnect clients;
- protocol snapshots can project a rich session and swarm status surface.

The proposed multi-surface client and custom desktop extend this topology, but they are not yet a
single verified, generally released workspace product in this audit.

### Akra

Akra deliberately delegates model and tool runtime authority to official `codex app-server`. Its
own domain is the operator workflow around that runtime:

- `src/core` coordinates app commands, effects, completions, streams, and snapshots;
- application services own conversations, planning, GitHub review polling, continuation, and
  parallel delivery;
- SQLite owns accepted planning, queue, leases, runtime events, and session detail;
- filesystem planning state is the operator-editable mirror;
- worktrees and GitHub adapters deliver parallel tasks into `prerelease`, with a reviewed PR path by
  default and explicit parent-level high-risk autonomous exceptions.

This is a narrower owned trust boundary but currently a weaker product story. It reduces the amount
of provider/tool/auth runtime Akra must defend, but this audit did not execute a comparative threat
model and therefore does not declare a security winner. A user sees jcode's one daemon as a
capability multiplier; Akra can look like a slower extra shell unless it projects Codex-native
capabilities and its delivery authority visibly.

Akra's session continuity also has a hard process boundary. Official thread history can be resumed,
but the connection owns the app-server child and terminates it on drop or transport failure. Runtime
reconnect launches a new child; this audit found no path that reattaches to the already-running turn
from the previous child. Current continuity is therefore thread and transcript continuity, not
client-independent active-turn survival.

### Decision

Keep `codex app-server` as the runtime authority. Do not build a parallel provider/tool daemon.
Instead, make the app-server boundary feel attachable and observable:

- preserve a typed, resumable core snapshot;
- consume more protocol-native execution events;
- expose live turn steering rather than waiting for a turn boundary;
- define active-turn exit, child-failure, and restart recovery without claiming unsupported
  background survival;
- measure the complete TUI plus app-server process tree;
- make planning, delivery, and review state visible without opening a sequence of overlays.

## TUI And Interaction Design

### Where jcode is ahead

jcode's TUI has a coherent instrumentation strategy. `WidgetKind` assigns an explicit priority,
preferred side, and minimum height to workspace, todos, context, memory, swarm, background work,
compaction, usage, cache, model, diagrams, ambient work, tips, and git. Dynamic state can raise a
widget's priority. This is more than adding panels: the layout decides what deserves scarce terminal
space.

The side panel is also an agent-facing capability. It can load or receive Markdown, follow a file,
act as a diff surface, take focus, and render diagrams. Session selection and TUI tests cover a broad
surface. Custom scrollback permits richer in-app behavior, while jcode openly acknowledges the
terminal limitation around smooth partial-line scrolling.

The strongest pattern is **progressive instrumentation**:

- common state remains small and persistent;
- richer detail is available without leaving the session;
- priorities change when usage, memory, or managed swarm work becomes urgent;
- the TUI can make parallel work feel spatial rather than presenting only a list of jobs.

### Where Akra is ahead

Akra's inline shell preserves host-terminal scrollback as durable history. Its terminal adapter,
scroll-region insertion, vt100 coverage, Windows Terminal/WSL validation method, and explicit
layering contract address real terminal behavior rather than assuming an alternate-screen app.

Akra also has a purpose-built supersession board and selected session timeline tied to its actual
parallel delivery states. It does not need to introduce generic widgets for every runtime subsystem.

### Where Akra is behind

Akra already reduces completed `commandExecution` and `fileChange` items into a coarse activity line
with current/last-turn counts and the latest summary. It still leaves higher-value `codex app-server`
notifications deferred or diagnostic-only, so the operator cannot see item start, command-output
deltas, patch progress, the aggregated turn diff, plan changes, or context pressure in that rail.
There is no protocol-native path for retaining the aggregated turn diff and inspecting it in a
drilldown. Running input also waits for the turn to finish even though the pinned protocol includes
`turn/steer`.

Akra's overlay-heavy feature growth can force the operator to ask "where do I open this?" rather
than seeing the next relevant fact. It needs a stricter information hierarchy:

1. conversation and host scrollback remain primary;
2. live execution and context pressure appear in a bounded status rail;
3. planning and parallel state use persistent compact projections when active;
4. detail views remain selected drilldowns, not permanent dashboard chrome;
5. narrow terminals collapse content by priority, not by arbitrary truncation.

The answer is not to port jcode's widget framework. It is to apply the priority discipline to
Akra-owned facts.

## Performance

### jcode's advantage

jcode makes startup and memory part of its public product identity. The repository includes PTY
startup/input measurement, PSS measurement, startup budget checks, memory regression gates, render
benchmarks, compile probes, code-size ratchets, and extensive TUI tests. This creates a strong
engineering feedback loop even when individual public claims need scrutiny.

The published values include 14.0 ms mean time to first frame, 48.7 ms mean time to first input,
27.8 MB PSS with local embeddings disabled, and about 9.9 MB additional PSS per session. Those
numbers are not accepted as current comparative fact here because:

- the memory table names jcode `v0.9.1888-dev`, not the audited v0.43.0 release;
- startup names only "this Linux machine" and ten PTY launches;
- raw JSON and a complete machine/terminal stamp are not tracked with the table;
- one competitor uses a different input-ready signal and was unauthenticated;
- daemon warm state and complete process-tree attribution are not sufficiently visible in the
  published table for an Akra comparison.

The correct conclusion is not "jcode is 63x faster than Akra." The correct conclusion is "jcode can
make a performance claim and Akra currently cannot."

### Akra's gap

Akra has real-terminal validation and a scheduler test that requires an immediate first frame, but
no repeatable benchmark for:

- process spawn to first visible shell frame;
- first frame to prompt echo;
- process spawn to app-server ready-to-submit;
- submit to first protocol event and first assistant delta;
- idle and streaming PSS for the complete Akra plus app-server process tree;
- memory scaling with resumed sessions and a three-slot parallel pool;
- frame cost and event backlog during dense tool output.

Until those are measured, "native" is an implementation detail, not a performance advantage.

### Required response

Akra should create an evidence contract before optimizing. It must store raw samples, exact binary
and Codex versions, auth state, daemon state, terminal geometry, environment stamp, process tree,
and p50/p95. Initial gates should ratchet against Akra's own baseline. Cross-product claims should
wait until the same harness can exercise both tools fairly.

## Parallel Work And Delivery

### jcode swarm

jcode's implemented and migrating swarm surface is ambitious:

- recursive spawning with parent/report-back ownership;
- direct messages, broadcasts, and scoped channels;
- lifecycle, activity age, current tool, tokens, todos, and completion report projection;
- same-repository read/change notification;
- a documented optional worktree-manager role, while automated worktree creation/integration was
  not verified in source;
- a legacy coordinator-gated shared plan plus live owner-partitioned DAG operations, with migration
  still in progress;
- a DAG model with ready, blocked, failed, cyclic, unresolved, confidence, and growth state.

This gives agents low-friction coordination and lets the TUI show what a group is doing now. The
asynchronous swarm-wait change in v0.43.0 is a good example of protecting coordinator responsiveness
as parallelism grows.

The README's stronger claim that all same-repository conflicts are automatically resolved is not
supported by the architecture contract. That contract describes optimistic, lock-free detection
followed by direct agent negotiation. Treat this as coordination assistance, not deterministic
conflict resolution.

Deep mode is also configured for unbounded recursion and per-node fan-out until a 1,000-member
swarm cap. The constant and cap checks are implemented, but this audit did not find or reproduce a
1,000-agent live load. That scale is both a capability and an operational cost/rate-limit risk.

### Akra parallel mode

Akra's parallel system is narrower but stronger at delivery. Its default path:

- accepted tasks come from operator-owned planning authority;
- one slot/worktree/branch owns a reviewable slice;
- leases and session detail are stored durably;
- completion refreshes official session truth;
- Git/GitHub adapters freeze a source range, push it, open a PR, verify review and checks against the
  frozen head, apply the range through a serialized integration worktree, push the integration ref,
  close the PR, and clean the slot;
- supervisor and distributor state are projected into TUI and Admin.

This is the fail-closed default, not the only configured path. With the explicit parent-level
high-risk autonomous-delivery opt-in, Akra skips approval, clean-merge, and required-check gates even
when it retains a PR. In eligible PR modes it can also skip PR automation and integrate directly.
Those exceptions preserve integration mechanics but weaken review evidence, so operator surfaces
must record the policy, PR presence, and skipped gates.

Worktree isolation prevents the same-checkout file-shift problem that jcode attempts to coordinate
through notifications. Akra should preserve that advantage.

### What to borrow

Borrow richer activity and completion semantics, not same-checkout collaboration:

- typed current tool and bounded recent activity for each slot;
- last-activity age distinct from lifecycle-state age;
- structured completion evidence rather than a free-form "done" summary;
- explicit failed reason and blocked dependency projection;
- preflight ownership/collision warnings when two planned slices name overlapping hotspots;
- a nonblocking wait path so selecting or supervising another slot never stalls the coordinator.

Do not add agent DMs as a primary UX. The operator, planning authority, and integration queue should
remain the coordination source.

## Memory And Continuity

jcode's memory system is a meaningful product investment. It has automatic extraction, semantic
retrieval, optional verification, explicit memory tools, session search, UI activity, and graph
directions. Core memory is documented as implemented, while the complete hybrid graph remains
planned.

Akra has durable planning and session continuity but no equivalent cross-session semantic memory.
Copying jcode's graph would be the wrong first move:

- embeddings add CPU, memory, packaging, privacy, invalidation, and relevance obligations;
- automatic user memory creates trust and correction requirements;
- `codex app-server` already owns conversation/session truth;
- Akra's strongest continuity primitive is accepted operator intent and delivery history, not an
  inferred personal memory graph.

Akra should first make provenance explicit:

- show which accepted direction, task, and prior session caused a continuation;
- index official Codex sessions and summaries without claiming replay fidelity for imported data;
- retain review feedback and delivery outcomes as structured operational history;
- expose superseded planning state rather than silently replacing it.

Only add semantic retrieval after a measured search failure and an explicit privacy/repair model.

## Providers, Tools, Hooks, And Customization

jcode's provider and auth breadth is an obvious acquisition advantage. A user can bring multiple
subscriptions and direct APIs, add compatible endpoints, use MCP, and connect lifecycle hooks. Its
self-development and reload model also appeals to users who want to reshape the harness itself.

For Akra, this is mostly a trap. Supporting provider transports, subscription credentials, model
catalogs, tool schemas, compaction semantics, and safety translations would duplicate the runtime
boundary and remove the product's clearest reason to exist.

The useful patterns are smaller:

- capability discovery should be explicit and machine-readable;
- optional integrations should not delay first input;
- lifecycle events should have bounded, structured payloads;
- hooks that can block execution need a clear fail-open or fail-closed contract;
- configuration diagnostics should explain credential source and active runtime without exposing
  secrets.

Akra should remain Codex-first and use official interfaces. Provider breadth belongs in upstream
Codex or a separately justified product, not in this wrapper.

## Admin, Desktop, And Remote Operation

jcode contains a substantial custom Rust desktop prototype and an ambitious spatial workspace
design, but the audited architecture documents still label the direction proposed. Its public
release path centers the `jcode` binary, not a verified general Admin site.

Akra already has a distinct advantage:

- Axum/Askama Admin and JSON API;
- a Game Development Country-inspired operational diorama;
- planning/draft/task/review pages;
- Telegram control;
- GitHub review polling and delivery state;
- the same application services underneath these inbound adapters.

The current parallel dashboard is a read-only projection, and the review center polls and displays
review state without owning the critical response loop. The weakness is therefore both truth
density and guarded actionability. Some Admin headline metrics and progress values remain
unmeasured or empty. Visual ambition without lifecycle-derived data will not beat a focused native
client.

Akra should evolve Admin into a real operations console:

- task throughput and outcome trends;
- queue wait, dispatch-to-running, running-to-report, and report-to-integration latency;
- p50/p95 rather than a single decorative speed number;
- explicit lifecycle stage `n/N` instead of invented percentages;
- selected agent activity, validation evidence, review state, and merge readiness;
- honest empty/error states.

After those projections are authoritative, guarded Admin actions should define shared
application-service commands for pause/resume, retry/handoff, and review response at the existing
control-plane boundary. HTTP handlers must not mutate SQLite or git directly.

The game layer should visualize actual company/work progression. It should never substitute for
operational truth.

## Safety And Trust

jcode's wide runtime makes safety difficult. It has permission UIs, tool policy, credential
diagnostics, safety design, and a synchronous pre-tool hook. The hook intentionally fails open for
timeout and unexpected failures. Ambient safety remains marked design, even though related runtime
code exists.

Akra's official app-server boundary and unattended-decline policy create a narrower structural
surface. Whether that is safer in practice remains an `inferred` hypothesis until both products are
evaluated against the same threat model. Akra should still make its verifiable policies visible:

- interactive approvals stay bounded and visible;
- hidden planning and parallel workers decline unsupported interaction;
- credentials remain owned by official Codex or explicit outbound adapters;
- remote writes require a verified GitHub identity;
- default-path delivery persists commit, PR, review, integration, and cleanup state, while the
  current `validation_summary` records planning-file-change context rather than command results;
- autonomous operator surfaces still need structured high-risk policy, PR-presence, and skipped-gate
  provenance.

The cost is less autonomy in ambiguous environments. That is acceptable. The product should be
trusted long enough to run, not merely powerful enough to surprise.

## Quality And Maintainability

jcode invests heavily in tests and ratchets. The audited workspace has more than 6,000 Rust test
markers and explicit budgets for size, panic-prone usage, swallowed errors, wildcard exports,
dependencies, compile performance, startup, and memory.

The scale debt is visible:

- approximately 518k production Rust LOC by this audit's coarse rule;
- 77 crates and many provider/runtime variants;
- numerous multi-thousand-line server, provider, tool, and TUI files;
- the v0.43.0 tag fails four checked-in quality ratchets locally, with 52 code-size regressions in
  one of them.

Akra is smaller but not small. It has approximately 205k production Rust LOC by the same coarse
rule and over 2,200 Rust test markers. Its architecture boundaries and TUI layering checks are an
advantage only if they keep feature work local.

The lesson is two-sided:

- adopt jcode's measurable budgets before Akra's surface grows further;
- do not copy jcode's breadth and then rely on ratchets to contain it.

## Strengths To Respect

- Performance and resource use are product-level concerns.
- The server/client model is designed to amortize multi-session runtime state and support reconnect.
- TUI instrumentation is prioritized, spatial, and state-aware.
- Memory and session search offer real continuity beyond a transcript list.
- Swarm status is rich enough for both agents and operators.
- Provider and customization breadth serves demanding power users.
- Tests, benchmarks, and guardrails cover an unusually wide native surface.

## Weaknesses To Exploit

- The runtime and credential surface is extremely broad.
- Shipped, migrating, proposed, and design-only concepts can be hard to separate.
- Performance tables are not pinned to the audited release with full raw evidence.
- Same-repository collaboration optimizes coordination more than protected integration.
- Desktop and ambient directions add large obligations before their release maturity is clear.
- The tagged source does not satisfy its own checked-in size ratchet.
- Broad provider compatibility makes official-protocol specialization harder.

## The Akra Wedge

Akra's sharp reason to exist should be expressed as one end-to-end promise:

> Start an official Codex session, steer it with protocol-native control, preserve accepted intent,
> observe every long-running lane, and finish with reviewed linear integration by default from one
> operator system.

That promise is narrower than jcode's and harder to replace with a generic harness. It is only
credible when Akra can prove:

- first frame, input, readiness, stream, and memory performance;
- complete live execution visibility without transcript noise;
- safe steering and approvals;
- explicit active-turn exit and restart recovery, with no duplicate turn submission;
- durable planning provenance;
- isolated parallel execution;
- validation, PR, critical review handling, serialized integration, and cleanup;
- explicit review-skipped policy provenance for every high-risk autonomous exception;
- real operational metrics in Admin.

The next work is therefore not "add more tools." It is to make the existing Codex-to-integration
loop measurably faster, more visible, and more trustworthy than a broad runtime can afford to be.
