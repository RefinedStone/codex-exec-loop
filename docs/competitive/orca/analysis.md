# Orca v1.4.137 Deep Dive

This analysis compares StablyAI Orca v1.4.137 with Akra at prerelease commit
`6274a7fc9703f85e4fb6247541dc0d4ed6c5fb9b`. Immutable source links, reproduced local observations,
artifact identity, and audit limits are in [evidence.md](evidence.md). Akra-relative decisions and
reviewable slices are in [gap-matrix.md](gap-matrix.md).

## Executive Verdict

The operator observation that Orca handles worktrees exceptionally well is verified. The important
innovation is not a faster wrapper around `git worktree add`. Orca promotes a worktree into the
primary application aggregate for a coding task:

```text
repository and base ref
-> worktree, branch, and ownership
-> agent terminal, editor, browser, and setup
-> diff, commit, push, pull request, and checks
-> archive, safe removal, or recoverable blocked state
```

Three implementation choices make that model unusually complete:

1. Git remains the authority for the live worktree inventory while Orca adds durable product
   metadata, ownership, links, lineage, terminal layout, and session state.
2. Removal is designed around failure states: dirty files, Git locks, nested worktrees, stale
   registration, orphan directories, moved branches, Windows file handles, and interrupted cleanup
   are explicit cases rather than one generic force flag.
3. The desktop UI and machine-readable CLI address the same worktree model across local, WSL, and
   SSH execution hosts.

Orca is therefore a serious threat to Akra's operator experience. It makes a changing fleet of
human-created workspaces easy to see, enter, resume, compare, and retire. Akra does not currently
offer an equivalent repository-wide worktree portfolio; its parallel board is intentionally scoped
to three runtime-owned slots.

That does not make Orca the stronger delivery authority. Orca's hosted-review path is an excellent
interactive Git client, but this audit did not find Akra's fixed lease generation, frozen remote and
source identities, host-owned bounded commit, reviewed-by-default gate with explicit parent-level
exceptions, serialized detached integration worktree, and remote-verification contract. Orca also
prefills high-autonomy arguments for supported agents on new launches unless global Manual mode or
an agent-specific override changes them. Official guidance justifies that default with a disposable
checkout whose diff can be reviewed or discarded. That is useful containment, but a Git worktree
isolates files and branches; it is not an OS, credential, network, process, or hook sandbox.

The strategic response is narrow:

- adopt Orca's authoritative repository-wide inventory and failure-state information design;
- preserve Akra's managed-slot, lease, security, and reviewed-delivery authority;
- add destructive worktree actions only after the read model proves ownership and staleness;
- reject a desktop/provider/terminal surface race and reject worktree-as-security-sandbox framing.

No fair same-machine end-to-end performance benchmark was run. The worktree verdict is a functional
and safety-design verdict, not a startup, memory, or latency claim.

## Product And Audience

Orca is not a terminal-only TUI. It is an Electron desktop Agent Development Environment with an
embedded terminal and a companion CLI. It targets developers who want to run multiple coding
agents in parallel, give each task an isolated branch and directory, retain its terminals and
workspace state, compare results, and send the selected result through source control.

Its reason to choose it is the cohesive workspace loop. A repository sidebar, worktree creation,
agent launch, terminal tabs, file and browser state, diff review, hosted pull-request state, and
cleanup all refer to one user-visible object. Official recipes explicitly demonstrate launching the
same prompt in three worktrees, comparing the diffs, shipping one, and deleting the others.

The surface is broader than the initial description suggests:

- local, WSL, and SSH repositories and terminals;
- multiple supported coding-agent CLIs;
- worktree-aware editor, browser, diff, and source-control panels;
- a JSON-capable CLI for repositories, worktrees, terminals, files, browser control, automation,
  orchestration, and computer-use operations;
- scheduled automation, experimental orchestration, mobile access, and session hibernation.

This breadth makes Orca an operator shell around agent CLIs. It does not replace those agents'
model or tool runtimes.

## Architecture And State Ownership

The audited release is an Electron application with a large main-process runtime, renderer, CLI,
terminal daemon, host providers, and relay code. The relevant ownership split is:

| State | Practical authority |
| --- | --- |
| registered worktree path, `HEAD`, branch, sparse and lock state | Git worktree inventory |
| Orca ownership, creation source, base, push target, links, lineage, comment | Orca metadata |
| panes, tabs, terminal bindings, scrollback, editor/browser layout | Orca persistence and terminal daemon |
| process and filesystem execution | local, WSL, or SSH host provider |
| PR, checks, review, and merge state | GitHub/GitLab plus Orca refresh coordinators |
| agent conversation/runtime semantics | selected agent CLI |

This is a useful composition. Orca does not pretend that a metadata row makes a path a worktree.
Fresh local discovery starts from `git worktree list`, joins product metadata afterward, and
invalidates cached scans after mutations. For a disconnected SSH host it may show stored metadata,
but marks the result non-authoritative rather than presenting cache as live Git truth.

The model is richer than path plus branch. Worktree metadata can include creation time and source,
base ref, push target, issue and pull-request links, agent state, sparse-checkout state, prior IDs,
and parent/child lineage with task or dispatch identifiers. The benefit is navigation and recovery;
the risk is a large reconciliation surface whenever Git or the filesystem changes outside Orca.

## Worktree Lifecycle

### Discovery, Ownership, And External Changes

Orca prefers NUL-delimited Git porcelain, parses path, `HEAD`, branch, bare, lock, and lock reason,
and falls back for older Git behavior. It annotates sparse-checkout state through separate bounded
probes. Concurrent scans are coalesced, and a mutation generation prevents a result started before
a create/remove operation from overwriting the newer view.

It classifies worktrees as Orca-managed, external, or unknown legacy state. Strong metadata and
known layout are ownership evidence; an arbitrary directory name is not. Externally created
worktrees can appear in an import inbox or remain visible according to repository policy. External
deletion is reconciled out of the live Git view on refresh. The scan prunes missing child lineage
and can rotate a missing parent's instance identity, but it does not immediately erase every durable
`worktreeMeta` entry.

The installed v1.4.137 runtime was pointed at this public Akra repository during the audit. It
reported the primary checkout, three detached Akra pool slots, and four branch worktrees exactly as
Git did. It detected the audit worktree created outside Orca without an application restart. This
is a sanitized observation; repository UUIDs, home paths, prompts, and terminal previews were not
retained.

### Creation, Branches, And Bootstrap

The user can start from a local branch, remote branch, or commit. Orca resolves and refreshes the
base when possible, handles branch/path collisions with bounded suffixing, and recognizes pull-
request or merge-request start points. Offline failure to refresh a remote does not erase an
already available local ref.

New local branches use `git worktree add --no-track -b`. Avoiding inherited tracking is a small but
important detail: the new task branch does not accidentally treat its base branch as the push
upstream. Orca records the actual base in branch configuration and, only when the user has not
already set it, may enable `push.autoSetupRemote` in shared repository configuration. That improves
first-push ergonomics but is a repository-wide side effect that should remain visible.

Creation also supports branch reuse, sparse checkout, shared/symlinked paths, setup commands, and
rollback when bootstrap fails. Setup comes from the newly created worktree's `orca.yaml` and runs in
a visible terminal. Visibility is better than a hidden bootstrap process, but executing repository-
controlled setup remains a trust decision; a worktree does not make hooks, filters, or commands
safe.

### Terminal, Session, And Workspace Continuity

Each worktree scopes terminal tabs and agent sessions. Stable worktree, pane, tab, and terminal
handles are injected into the terminal environment so CLI automation can address the active
workspace. PTY bindings and a minimum layout are flushed before spawn completion, narrowing the
window in which a process exists without recoverable UI ownership.

The terminal daemon can keep processes and scrollback alive after the desktop renderer closes.
After a machine reboot, processes are gone but layout and saved scrollback can be restored. Session
hibernation is explicitly experimental and depends on an agent's resume primitive. These are three
different continuity claims and should not be conflated:

- renderer/app detach continuity;
- persisted layout and scrollback restoration;
- agent-process or conversation resume.

Persistence partitions local, SSH, and remote-runtime hosts. WSL shares the local session partition
while retaining explicit WSL Git, path, cache, and environment routing. SSH uses relay RPCs to
create and inspect actual remote worktrees and to keep terminal, file-event, and diff state
associated with the remote workspace during its configured disconnect policy. The audited source
defaults an established relay to “until reset” and also supports bounded 60-second through seven-day
timeouts; reconnect continuity is therefore policy-scoped, not an unconditional process-survival
claim. The current official SSH page instead says the default grace is five minutes. This analysis
binds implementation claims to the v1.4.137 tag and records that documentation/source mismatch.

### Removal And Recovery

Removal is the standout implementation.

Before destructive action, Orca resolves a registered target canonically from Git rather than
trusting a renderer-supplied path. It rejects the primary checkout, dangerous locations, nested
worktrees, and Git-locked worktrees. A registered external worktree can be removed without Orca
ownership metadata; ownership and `.git` backlink proof become mandatory for unregistered/orphan
recursive recovery. A normal delete checks tracked and untracked dirtiness before attempting to
tear down worktree-owned PTYs. This ordering matters: a failed preflight does not destroy the user's
still-useful terminal context.

After preflight it makes a best-effort attempt to close only the target worktree's PTYs and watchers,
which is particularly important for Windows file handles; resource-close errors are logged and Git
removal can continue. Local branch deletion then uses `git branch -d`, so an unmerged or unpublished
branch is preserved with its exact `HEAD` instead of being silently discarded. Force applies to
`git worktree remove --force`, not to branch CAS. A separate later action that retires a preserved
branch can use `git update-ref -d` with the previously observed expected `HEAD`, so a branch that
moved after confirmation survives.

Recovery distinguishes several partial states:

- stale Git registration that can be pruned;
- an Orca-owned orphan directory whose provenance can be proved;
- Git for Windows removing registration but failing to delete files;
- metadata whose instance identity no longer matches the live worktree;
- an already removed worktree with a branch that must still be preserved.

The product value comes from keeping these states explainable and retryable. It is materially safer
than a generic `--force` confirmation dialog.

## Parallel Work And Coordination

Orca's primary parallel pattern is dynamic, human-directed fan-out: create as many worktrees as the
operator wants, start agents, compare outcomes, and choose what to ship. It also exposes worktree
selectors and JSON output so agents and automations can create, inspect, comment on, and address
workspaces.

Its experimental orchestration layer adds persistent messages, tasks, dispatch records, decision
gates, coordinator concurrency, and worker completion identity. Scheduled automations can target a
repository or reuse an existing worktree. Worktree comments can act as lightweight checkpoints.

These are valuable control surfaces, but they are not equivalent to Akra planning authority. A
free-text checkpoint is useful operator context, not completion proof. Experimental orchestration
and dynamic worktree count do not establish durable capacity leases, stale-generation rejection,
or reviewed delivery.

Akra intentionally chooses a different automation model: a fixed three-slot pool, repository-
scoped OS mutation lock, SQLite lease authority, 64-hex generation CAS, frozen delivery target,
unattended worker policy, and one serialized distributor. Orca is better at ad hoc workspace fleet
ergonomics; Akra is stronger at bounded autonomous ownership.

## Git, Hosted Review, And Delivery

Orca supplies a cohesive interactive source-control path: staged and hunk-level diff review,
commit, push, existing-PR association, draft creation, checks, reviews, comments, and merge. It
refreshes PR state with host-scoped coordination and rate/backoff policy. Pull-request creation
rechecks detached/default branch, dirty state, upstream, ahead/behind state, authentication, and
existing matches immediately before the side effect. Fork PR start points fetch and verify the
remote ref.

The implementation avoids several common footguns:

- normal push does not silently force;
- explicit force uses a lease-aware path;
- ambiguous PR creation can be reconciled by head/base lookup instead of duplicated;
- merge does not ask the provider to delete the branch, leaving branch retirement to the safer
  worktree lifecycle;
- a terminal-observed PR URL is linked only after branch association is verified.

This is strong interactive Git/PR tooling. It is not the same contract as Akra's distributor. The
audit did not find one durable authority chain that freezes the remote URL, repository visibility,
integration OID, exact source SHA and commit range before work; requires a matching reviewed PR
head and passing gates by default; serializes integration in a detached worktree; verifies the
remote integration ref; and cleans the source branch with frozen-SHA compare-and-swap.

Orca optimizes “help the operator ship this worktree.” Akra optimizes “prove that this accepted task
became this reviewed remote result.” The latter is the differentiation to preserve.

## Safety And Trust Boundaries

Orca prefills high-autonomy arguments for supported agents on new launches, including Codex's
approval/sandbox bypass flag. Global Manual mode and agent-specific overrides can change that
behavior. Official guidance explains the default in terms of a disposable checkout whose diff can
be reviewed or discarded. That containment argument is not a security boundary and is too broad as
an Akra permission policy.

A worktree provides a separate checkout and branch. It does not isolate:

- process, port, IPC, or network namespaces;
- user credentials, environment variables, or home-directory files;
- repository hooks, filters, tools, and setup commands;
- shared Git configuration, object database, refs, remotes, or credential helpers;
- services and databases reached by the agent.

Orca's deletion safeguards reduce accidental file loss; they do not convert the workspace into a
security sandbox. Akra should keep official app-server approval semantics for the main session,
fail closed for unattended approvals, scrub child environment by default, audit effective Git
execution configuration, and use isolated frozen network delivery.

Orca documents that prompts, file contents, terminal output, and path names are excluded from
product telemetry. What the official page calls anonymous product-usage telemetry can include fixed
enums, version strings, and a locally generated random identifier, and goes to PostHog in the US.
Telemetry can be disabled. Orca documents PostHog plan-level default retention and no custom
retention window; this audit did not independently inspect network traffic, remote-agent providers,
mobile transport, retention enforcement, or every third-party agent CLI.

## Performance, Quality, And Maintainability

The worktree area has extensive unit and end-to-end test code for scanning, collisions, NUL and
newline paths, old Git behavior, Windows partial deletion, ownership forgery, persistence, terminal
scope, PR start points, and refresh coordination. That is meaningful evidence of engineering
investment, not proof that every packaged path passed for v1.4.137.

The public tag was inspected without dependencies, so its tests were not executed. The installed
binary and CLI were exercised only for identity, daemon readiness, and sanitized worktree
inventory. No cold/warm launch samples, input-to-render samples, worktree-create latency, memory,
disk growth, or complete Electron/daemon/agent process-tree measurements were captured. Akra and
Orca therefore have no comparable performance winner in this audit.

The architecture cost is a very large Electron/TypeScript surface with local, WSL, SSH, desktop,
mobile, CLI, browser, GitHub, GitLab, and multiple agent paths. Its rapid release cadence increases
the need to bind claims to an exact stable tag. Akra should learn from the lifecycle model without
copying this surface area.

## Akra Comparison

| Axis | Orca v1.4.137 | Akra `6274a7fc` | Verdict |
| --- | --- | --- | --- |
| primary unit | dynamic human-visible worktree workspace | accepted task plus fixed leased slot and delivery record | different aggregates |
| repository inventory | managed and external Git worktrees, import and reconciliation | three managed pool slots and dedicated integration state | Orca ahead in portfolio UX |
| terminal continuity | worktree-scoped PTY/layout across app detach; host-scoped persistence | official thread/session resume and one inline shell; no repository-wide worktree jump | Orca pattern worth adopting narrowly |
| delete safety | canonical Git target, lock/dirty/order checks, branch preservation, orphan recovery | identity-checked slot reset/cleanup after integration; ordinary worktrees remain outside Akra authority | both strong in different scopes |
| automation capacity | dynamic workspaces; experimental orchestration | fixed three-slot capacity with durable lease generation and dispatch authority | Akra ahead for bounded automation |
| worker boundary | new launches prefill high-autonomy arguments unless Manual/agent override changes them | unattended worker uses workspace-write, approvals decline, host validates/prepares bounded commit state | Akra ahead |
| delivery | interactive commit/push/PR/check/review/merge | frozen source/target, reviewed default, serialized integration, remote verification | Akra ahead in authority |
| remote workspace | first-class SSH relay plus local/WSL | local pool; remote node work is not equivalent | Orca ahead in shipped workspace reach |
| performance evidence | no comparable audit sample | no comparable complete process-tree baseline | unknown |

## Decisions

### Adopt

- A read-only, Git-authoritative repository worktree portfolio that includes Akra-managed slots,
  the integration worktree, other registered worktrees, and external/unknown entries.
- Separate registration authority, status freshness, managed binding, ownership, lock, dirty,
  branch/`HEAD`, and optional owner-supplied link fields rather than inferring health from a
  directory name or one authority boolean.
- External-change reconciliation and an operator-visible stale/orphan state before any import or
  removal action exists.
- Orca's destructive-action ordering if Akra later owns manual retirement: prove the Git target and
  ownership, inspect lock and dirty state, then stop scoped runtime resources, remove, preserve a
  moved/unpublished branch with expected-`HEAD` CAS, and reconcile.
- A compact worktree detail flow and linked-session selection backed by the same application read
  model after the inventory contract is stable. Changing official thread/workspace context remains
  a separate P2 decision.

### Reject

- Worktree isolation as justification for bypassing approvals or sandboxing.
- Repository-controlled setup, hooks, filters, or shared Git config changes without Akra's current
  audit and explicit operator policy.
- Free-text checkpoints as task, lease, completion, review, or delivery authority.
- Unbounded dynamic worktrees as a replacement for fixed unattended capacity.
- A multi-agent-provider desktop IDE, mobile client, browser editor, or generic terminal-control
  plane as Akra's product strategy.
- Provider-default merge and branch cleanup as a replacement for Akra's frozen reviewed-range
  patch comparison and cherry-pick integration delivery.

### Differentiate

Keep Akra's stronger chain visible:

```text
accepted DB task
-> fixed-capacity slot with OS mutation lock and generation lease
-> official app-server worker in an exact worktree
-> host-owned bounded commit validation/preparation
-> frozen remote, repository, base, source, and commit range
-> matching PR, review, checks, and mergeability by default
-> serialized detached integration and remote verification
-> identity-checked source and slot cleanup
```

Orca should own the benchmark for worktree fleet ergonomics. Akra should own the benchmark for
Codex-first reviewed outcome authority, while adopting only the repository inventory and recovery
information that makes that authority easier to operate.

## Refresh Triggers

Refresh this analysis when Orca changes its worktree ownership/removal model, graduates
orchestration or hibernation, changes agent permission defaults, ships a material terminal-daemon
architecture change, or reaches a new major release. Refresh the comparison when Akra ships a
repository-wide worktree inventory or manual worktree actions.

## Sources

- [Orca repository](https://github.com/stablyai/orca)
- [v1.4.137 release](https://github.com/stablyai/orca/releases/tag/v1.4.137)
- [Worktrees](https://www.onorca.dev/docs/model/worktrees)
- [Agents and sessions](https://www.onorca.dev/docs/model/agents-sessions)
- [Supported agents and permissions](https://www.onorca.dev/docs/agents/supported)
- [Session restore](https://www.onorca.dev/docs/model/session-restore)
- [SSH](https://www.onorca.dev/docs/ssh)
- [CLI overview](https://www.onorca.dev/docs/cli/overview)
- [Orchestration](https://www.onorca.dev/docs/cli/orchestration)
- [Commit and push](https://www.onorca.dev/docs/review/commit-push)
- [Hosted reviews](https://www.onorca.dev/docs/review/github)
- [Telemetry](https://www.onorca.dev/docs/telemetry)
