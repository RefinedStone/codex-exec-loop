# Orca To Akra Gap Matrix

[한국어 번역](../../ko/competitive/orca/gap-matrix.md)

This matrix compares Orca v1.4.137 at
`6013055491943336660e12e5dec93c9ece4575bb` with Akra 1.3.5 at
`6274a7fc9703f85e4fb6247541dc0d4ed6c5fb9b`. It is a product-relative decision source, not a total
score. Evidence and limits are in [evidence.md](evidence.md).

## Relative Matrix

| Capability | Orca v1.4.137 | Akra `6274a7fc` | Decision | Priority | Evidence |
| --- | --- | --- | --- | --- | --- |
| Product authority | desktop ADE coordinates agent CLIs, worktrees, terminals, editor/browser, and source control | Codex-first application and delivery authority on official `codex app-server` | keep Akra thesis; adopt only operator patterns | invariant | [topology](evidence.md#product-release-and-topology), [Akra](evidence.md#akra-baseline-evidence) |
| Worktree aggregate | worktree is the top-level task workspace joining branch, agent, terminal, files, diff, PR, and lineage | accepted task, lease, official session, source, and delivery record are authority; slot is a resource | expose a portfolio without replacing task authority | P1 | [model](evidence.md#worktree-discovery-and-ownership) |
| Repository inventory | managed, external, and legacy worktrees; Git-authoritative refresh and import visibility | board joins only the generated three-slot pool and integration state | add a repository-wide read model | P1 | [runtime observation](evidence.md#audit-environment-and-privacy), [Akra](evidence.md#akra-baseline-evidence) |
| Base selection | local/remote branch or commit, refresh with offline fallback | fetched configured integration branch/OID is the automation baseline | keep strict remote proof for automation; allow read-only display of arbitrary bases | invariant/P1 | [creation](evidence.md#creation-bootstrap-and-metadata) |
| Creation and collision UX | dynamic path/branch creation, bounded suffixing, branch reuse, PR start point, sparse mode | fixed generated sibling paths, detached baseline slots, generated agent branches | do not replace the fixed pool; learn from explicit conflicts for future manual actions | shipped/contingent | [creation](evidence.md#creation-bootstrap-and-metadata), [Akra](evidence.md#akra-baseline-evidence) |
| Ownership | managed/external/unknown plus instance metadata and provenance | generated pool path, branch prefix, exact lease generation, canonical/link-free identity | project both product ownership and authority generation | P1 | [ownership](evidence.md#worktree-discovery-and-ownership), [cleanup](evidence.md#akra-baseline-evidence) |
| Bootstrap | visible terminal setup from worktree `orca.yaml`; sparse/shared paths | workers run within a pre-provisioned slot; host audits Git execution config and owns commit | reject implicit repository-controlled setup in unattended authority | invariant | [bootstrap](evidence.md#creation-bootstrap-and-metadata) |
| State display | sidebar plus branch, agent, terminals, diff, PR, comment, lineage, external state | TUI board shows readiness, pool, roster, selected lifecycle, queue, and delivery boundary | add repository portfolio detail; keep task/delivery truth primary | P1 | [analysis](analysis.md#worktree-lifecycle), [Akra](evidence.md#akra-baseline-evidence) |
| Worktree jump and resume | worktree-scoped tabs/layout/session, CLI selectors, local/WSL/SSH host scope | session browser resumes official threads in current workspace context; no general worktree jump | add only after workspace/thread authority is explicit | P2 | [continuity](evidence.md#terminal-session-and-remote-continuity) |
| Parallel capacity | dynamic human-created worktrees; experimental coordinator concurrency | fixed three-slot pool with durable dispatch and capacity decision | differentiate on bounded unattended capacity | shipped | [automation](evidence.md#automation-permissions-and-telemetry), [Akra](evidence.md#akra-baseline-evidence) |
| Isolation | separate branch/directory; new agent launches prefill high-autonomy arguments unless Manual/agent override changes them | separate slot plus workspace-write worker, declined unattended approvals, scrubbed environment, audited Git boundary | reject worktree-as-security-sandbox | invariant | [permissions](evidence.md#automation-permissions-and-telemetry) |
| Dirty, lock, and nested state | parses locks; blocks normal dirty/untracked removal; rejects nested and unsafe targets | reset/cleanup blocks dirty, untracked, pending, non-integrated, and identity-mismatched state; exact post-integration cleanup alone may purge ignored build output | preserve Akra fail-closed checks; adopt compact reason vocabulary | shipped/P1 UX | [Orca removal](evidence.md#removal-and-recovery), [Akra](evidence.md#akra-baseline-evidence) |
| Destructive ordering | re-resolve target, check lock/dirty, then attempt scoped PTY/watcher close and remove; close failure is best effort | pool cleanup proves lease/path/branch/integration and does not own arbitrary PTYs | use the preflight ordering, but choose an explicit fail/continue policy for Akra | contingent | [removal](evidence.md#removal-and-recovery) |
| Branch preservation | `branch -d` preserves moved/unpublished history; a separate later preserved-branch action can use expected-`HEAD` CAS, while forced worktree removal itself does not | local/remote cleanup compares frozen identity and preserves moved branches | equivalent safety principle with different action boundaries; keep Akra proof | shipped | [Orca removal](evidence.md#removal-and-recovery), [Akra](evidence.md#akra-baseline-evidence) |
| External mutation and orphan recovery | import/reconcile, stale registration, orphan provenance, Windows partial deletion | conservative pool reconciliation preserves unproved paths and blocks split brain | add external projection first; do not auto-import or delete | P1 | [recovery](evidence.md#removal-and-recovery) |
| Base drift | post-create base reconciliation and on-demand behind probe; unknown is non-blocking and SSH probe is unknown | newly advanced remote baseline blocks mutation; frozen target and exact range gate delivery | Akra ahead for automation authority | shipped | [Orca drift](evidence.md#base-and-drift-reconciliation), [Akra](evidence.md#akra-baseline-evidence) |
| Commit/source freeze | interactive worktree and provider state | host-owned bounded commit plus exact source SHA and 1-128 linear commit range | differentiate | shipped | [Akra](evidence.md#akra-baseline-evidence) |
| PR and review | strong interactive commit/push/draft PR/check/review/comment/merge path | default approval/check/clean/matching-head gates and serialized integration | adopt deep-link UX; preserve Akra authority | shipped plus existing delivery work | [Orca PR](evidence.md#git-hosted-review-and-delivery), [Akra](evidence.md#akra-baseline-evidence) |
| Cleanup completion | workspace/archive lifecycle with safe local branch handling | remote integration verification, PR close, source compare-and-delete, slot baseline cleanup | differentiate on terminal delivery proof | shipped | [Orca removal](evidence.md#removal-and-recovery), [Akra](evidence.md#akra-baseline-evidence) |
| Remote workspace | first-class SSH worktrees, relay PTYs/files/diff, non-authoritative offline fallback | local fixed pool; Admin/Telegram are control surfaces, not remote workspaces | do not start a generic SSH IDE lane; reuse future node contract only if product demand exists | reject/current | [remote continuity](evidence.md#terminal-session-and-remote-continuity) |
| Performance | no comparable packaged process-tree sample | no comparable versioned process-tree/worktree sample | winner unknown; amend existing evidence contract | existing P0 | [limits](evidence.md#audit-limits), [existing owner](../jcode/gap-matrix.md#p0-native-performance-evidence-contract) |
| Release quality | extensive worktree-focused static tests; not run in this audit | broad Rust tests and architecture gates; exact release validation remains portfolio work | bind all claims to exact source and raw validation | existing P0 | [quality](evidence.md#quality-evidence), [existing owner](../jcode/gap-matrix.md#p0-frozen-source-validation-evidence) |

## Portfolio Rules

Orca supplies one genuinely new Akra product gap: repository-wide worktree fleet visibility. Other
lessons amend existing work rather than creating parallel backlog names.

| Orca lesson | Owning Akra item | Amendment only |
| --- | --- | --- |
| worktree/terminal/diff/PR status should remain one drilldown | [Protocol-Native Live Execution Rail](../jcode/gap-matrix.md#p0-protocol-native-live-execution-rail) and [Authoritative Delivery Outcomes](../jcode/gap-matrix.md#p1-authoritative-delivery-outcomes) | add worktree ownership and path/branch link fields; do not merge runtime activity with delivery authority |
| worktree-scoped agent/session lineage | [Official Session Search And Provenance](../jcode/gap-matrix.md#p2-official-session-search-and-provenance) | include explicit workspace identity and missing/moved workspace state |
| scheduled and event-driven launch | [Automation Intake Core](../agent-canvas/gap-matrix.md#p1-automation-intake-core) | retain idempotent typed intake and lease acquisition; do not create an Orca-specific automation lane |
| create/list/switch/remove performance | [Native Performance Evidence Contract](../jcode/gap-matrix.md#p0-native-performance-evidence-contract) | add exact worktree-count, cold/warm Git state, disk, process-tree, and raw samples |
| exact stable tag despite rapid releases | [Frozen Source Validation Evidence](../jcode/gap-matrix.md#p0-frozen-source-validation-evidence) | bind worktree lifecycle tests and captures to the release SHA |
| command-addressable worktree selectors | [Contextual Command Discovery](../opencode/gap-matrix.md#3-contextual-command-discovery) | expose availability and authority reason through Akra's existing registry; do not add a second command system |

There is no new P0. Orca does not displace the existing live rail, recovery, automation, performance,
validation, or delivery owners. The new inventory begins at P1 because Akra's current three-slot
automation remains correct without it.

## Adopt

### 1. P1 Authoritative Worktree Portfolio Read Model

Add a read-only application model for all worktrees in the current repository. It must not live as
a TUI-only `git` call.

**Owned boundary**

- a domain DTO for repository/worktree identity, observation generation, and field-group freshness;
- an outbound port for authoritative Git inventory and bounded status inspection;
- the Git adapter for porcelain parsing and platform path normalization;
- an application service that joins Git inventory only with the current managed slot and integration
  identities from existing parallel authority;
- typed TUI projection. Session/activity and delivery owners may later contribute optional typed
  links; this slice neither rebuilds nor persists their state.

**Minimum fields**

- repository identity derived from the canonical common Git directory, without exposing that raw
  path through remote DTOs;
- Git worktree identity is a tagged result: `resolved` combines the canonical Git-reported path and
  resolved per-worktree Git directory; `unresolved` retains only bounded Git-reported spelling,
  common-directory admin-entry evidence when available, and an explicit failure reason. Display
  spelling is never silently promoted into identity;
- a monotonic observation generation and observed-at time so a response started before a refresh
  cannot replace the newer snapshot;
- separate `registration_authority`, `status_freshness`, and `managed_binding` fields. A live Git
  inventory, a timed-out status inspection, and an exact SQLite lease-generation match are not one
  boolean;
- ownership: `primary`, `managed_slot`, `managed_integration`, `observed_external`, or
  `unknown_legacy`; ordinary registration alone never proves a human owner;
- Git registration, branch or detached state, `HEAD`, lock and lock reason;
- staged, unstaged, untracked, ignored, pending-operation, and submodule summary with explicit
  unknown values when inspection is bounded or fails;
- matching Akra slot and exact lease generation or integration identity when available. Task,
  activity, official session, source, PR, and delivery detail remain optional links owned by their
  existing application projections;
- no arbitrary terminal or prompt contents.

**Initial bounds**

- at most 512 inventory records and 8 MiB of NUL-delimited Git output per refresh;
- at most 32 KiB of encoded path data per record;
- status inspection for at most 64 worktrees per refresh, prioritizing selected and Akra-managed
  rows, with no more than eight commands in flight;
- at most 1 MiB or 50,000 status records per inspected worktree and 16 MiB of retained status output
  across one refresh;
- two seconds per status command and ten seconds for the aggregate refresh;
- explicit `truncated`, `not_inspected`, `timed_out`, and `failed` states. A partial/truncated result
  is useful orientation but is not authoritative proof for a future action.

**Safety contract**

- Git is authority for registration; SQLite remains authority for leases/tasks/delivery;
- a path match without canonical, link-free identity does not confer Akra ownership;
- an unresolved missing/prunable row may be displayed from Git common-directory admin evidence, but
  it cannot authorize start, resume, import, reset, prune, or deletion;
- list and refresh are read-only and never provision, reset, import, prune, kill, or delete;
- use the existing pinned and scrubbed host-Git boundary with optional locks disabled; inspection
  may run only bounded list/status/identity commands and cannot execute repository hooks, filters,
  tools, credential helpers, or prompts;
- an unavailable or truncated Git inventory cannot be represented as live registration authority;
- bounded inspection failures remain visible per row rather than dropping the row;
- an externally removed worktree disappears on the next complete authoritative refresh. This slice
  stores no tombstone for arbitrary external paths. A missing managed slot may remain visible only
  because the exact SQLite/pool owner still projects it as missing;
- no private absolute path enters remote/Telegram projection by default; Admin and TUI may show a
  local display path under their existing authenticated/local boundaries.

**Proof**

- fixtures for primary, detached slot, integration, ordinary branch, external worktree, locked
  worktree, newline/space path, separate Git dir, symlink/junction alias, and stale registration;
- an integration test creates and removes an external worktree outside Akra and proves refresh adds
  then removes it without process restart, tombstone creation, or pool mutation;
- a missing managed slot remains as an exact authority-backed blocked row while an arbitrary missing
  external path does not;
- limit, timeout, concurrency, partial-result, stale-generation, and cancellation tests exercise
  every initial bound;
- a generation replacement test proves a stale row cannot bind to the new slot lease;
- before/after captures prove refs, worktree indexes, repository/worktree config, and SQLite pool
  authority are byte-identical across list and refresh;
- architecture gates prove inbound adapters depend on the service projection, not Git adapters.

### 2. P1 Worktree Portfolio TUI

After the read model is green, add one compact searchable overlay from the existing command
registry. It should answer four questions without becoming a file browser:

1. Which worktrees exist?
2. Which are Akra-owned versus merely observed?
3. Which currently shipped managed lifecycle state is linked to each managed worktree?
4. Why is a worktree stale, dirty, locked, blocked, or safe only for manual inspection?

The default row should fit branch/detached identity, ownership, dirty/lock marker, and the coarse
managed lifecycle already available from the pool projection. Selected detail can show the local
display path, exact `HEAD`, lease generation, status breakdown, and recovery guidance. Optional
activity, session, source, PR, and delivery links render only when their existing owning projection
supplies them; otherwise the UI says unavailable. Reuse the current overlay/list and parallel
detail composition; do not create a general tree widget framework.

The first slice is inspect-only. It may open the already shipped parallel detail, copy/show the
local path, or print a safe command. Selecting or changing an official session, active app-server
workspace, or shell cwd remains a separate P2 because it can change thread context and runtime
ownership.

**Proof**

- narrow/wide snapshots with mixed managed, external, locked, dirty, and stale rows;
- keyboard search/selection and exact command-registry availability tests;
- a real-terminal capture showing an externally created worktree appearing on refresh;
- refs, indexes, Git config, and pool authority remain byte-identical while opening, refreshing,
  searching, and closing the overlay.

### 3. P1 Admin Worktree Portfolio Projection

After the application read model is stable, expose a separate read-only Admin DTO and endpoint from
the same service projection. The handler owns authentication and presentation only.

- allowlist every field; return a bounded local label by default rather than the raw absolute path;
- retain separate registration, status, and managed-binding freshness in JSON;
- represent truncated and unavailable inventory explicitly;
- expose optional session/delivery links only after their owning projections ship them;
- add valid-session, missing/invalid capability, origin, path-redaction, truncation, and no-mutation
  API tests;
- do not add import, reset, prune, terminal, or deletion actions.

### 4. P2 Worktree-Scoped Session Jump

Extend the existing session provenance owner rather than inventing an Orca-style session database.
This slice first adds an interactive-target capability, not a new ownership class:

- `primary` is eligible for interactive start/resume;
- `managed_slot` remains owned by its exact parallel lease and cannot be entered through this action;
- `managed_integration`, `unknown_legacy`, unresolved, missing, and remote rows are never eligible;
- `observed_external` is inspect-only until a local operator explicitly selects its full current
  repository/worktree identity. That action mints a one-request, process-local capability bound to
  the observation generation; it grants no deletion, setup, automation, lease, or durable ownership;
- immediately before use, the service reruns identity resolution and rejects a changed generation,
  path, Git directory, repository, or registration state.

Starting and resuming then have different authority:

- start a new official thread by passing the selected current worktree cwd through the existing
  start request and then revalidating the applied cwd;
- resume a linked thread by loading its official persisted cwd through the existing resume path and
  requiring that cwd to map canonically, without a link/reparse escape, to a current inventory
  identity. The selected row never overrides the thread's persisted cwd;
- inspect an ineligible workspace without starting anything.

Do not silently `chdir` the current conversation, retarget a live worker, or reinterpret a thread
created in another checkout. A worktree path is context, not thread authority.

**Proof**

- source and target workspaces retain distinct official thread IDs and transcript histories;
- managed slot/integration and unresolved/unknown rows cannot mint an interactive-target capability;
- an external capability is single-use, generation-bound, and cannot authorize any Git, pool, or
  delivery mutation;
- a moved/deleted selected worktree blocks start, and a missing/mismatched persisted thread cwd
  blocks resume while keeping history inspectable;
- approval and interrupt requests remain bound to the exact official thread/turn and cannot resolve
  a runtime in another worktree.

### 5. Contingent Manual Retirement Contract

Do not add delete/import/archive buttons with the read model. If a later operator use case justifies
Akra-owned manual retirement, use a separate reviewed slice with this order:

```text
refresh authoritative Git inventory
-> prove canonical target and explicit Akra/manual ownership
-> reject primary, nested, linked/aliased, locked, dirty, pending, or unknown state
-> capture expected branch HEAD and confirm the exact consequence
-> stop only resources proven to belong to that worktree
-> ask Git to remove it
-> delete a branch only with safe ancestry or expected-HEAD CAS
-> preserve moved/unpublished history
-> reconcile and persist a terminal outcome
```

Never recursively delete an orphan because its path looks generated. Proven Git backlink, common
admin entry, application provenance, canonical path identity, and a bounded recovery action are all
required. Windows partial removal and Git registration prune must be distinct outcomes. This is a
future safety contract, not accepted implementation work from this analysis.

## Reject

### Worktree As Security Sandbox

Do not pass approval/sandbox bypass flags merely because the cwd is a worktree. Preserve Akra's
main-session approval projection, unattended decline policy, child-environment filtering, effective
Git-config audit, pinned executables, and frozen isolated delivery context. A worktree does not
contain credentials, processes, networking, hooks, filters, or shared Git state.

### Repository-Controlled Unattended Bootstrap

Do not automatically execute `orca.yaml`-style setup, hooks, package scripts, or arbitrary commands
before a worker owns a slot. If Akra later needs bootstrap, it requires an explicit operator policy,
immutable command identity, audited environment, bounded time/output, visible provenance, and no
authority mutation on failure. The current fixed reusable pool avoids most of this need.

### Free-Text Or Client Metadata Authority

Comments, labels, colors, pane names, terminal output, and agent self-reports may improve operator
context but cannot become task completion, lease ownership, review, integration, or cleanup proof.
Keep typed DB authority and official/runtime evidence.

### Dynamic Capacity As Default Automation

Do not turn every observed developer worktree into an automation lane. Fixed capacity makes
resource demand, lease generation, cleanup, and serialized delivery auditable. An external
worktree remains read-only until an explicit future import contract proves its owner and target.

### Surface And Provider Race

Reject copying Orca's Electron IDE, generic browser/editor, mobile control, multi-agent launcher,
computer-use surface, or SSH relay as a response to this analysis. Akra's TUI, app-server runtime,
Admin, CLI, Telegram, and automation should continue to share application services. Remote node
work remains separately justified and read-only first.

### Interactive Git State As Delivery Authority

Do not replace frozen source/target identity, review/check gates, detached integration, remote
verification, and compare-and-delete cleanup with “the current worktree looks pushable” or provider
default merge behavior. Orca's PR UX can inspire links and diagnostics; Akra's distributor remains
the terminal authority.

## Differentiate

### Dynamic Workspace UX Versus Bounded Automation Authority

Orca can be best for a developer who wants to fan out arbitrary agents and move among their living
workspaces. Akra should be best for an operator who wants an accepted Codex task to consume bounded
capacity and reach a reviewed, identity-proven remote outcome. The portfolio view bridges the UX
gap without erasing the distinction.

### Frozen Reviewed Outcome

Make the stronger Akra evidence legible in every managed-worktree detail:

- exact lease generation and current lifecycle owner;
- frozen integration base, source branch and source SHA/range;
- official completion versus host commit readiness;
- push, PR, approval/check/mergeability, integration, remote verification, and cleanup outcomes;
- explicit high-risk autonomous-policy provenance when the reviewed default was bypassed.

Orca shows why one cohesive lifecycle is valuable. Akra can make that lifecycle more accountable by
projecting durable authority instead of client metadata.

### Conservative Reconciliation

Orca has broader orphan recovery; Akra has the right default for automation: if a path or generation
cannot be proved, preserve it and block. The portfolio should make that conservative result easy to
understand. Recovery guidance is an operator feature, not evidence that automatic deletion is safe.

### Codex-Native Session Context

Worktree navigation should retain official thread and turn identity rather than treating terminal
scrollback or a CLI resume token as the conversation source of truth. Akra can combine Orca-quality
workspace navigation with protocol-native provenance.

## Comparative Experiments

These experiments use disposable repositories and raw artifacts. They do not block the read-only
inventory slice, and they do not create separate product priorities.

### Worktree Lifecycle Fixture

For both products, capture exact versions and Git state before and after:

- first clean worktree creation;
- three simultaneous worktrees and branch/path collision;
- tracked, untracked, ignored, locked, nested, and pending-operation states;
- external add/remove, stale registration, and moved branch;
- app termination between creation/removal phases;
- base advance, rebase conflict, integration, and final cleanup;
- spaces and newlines in paths where the platform supports them;
- Windows file-handle partial deletion and WSL path spelling.

Each transition records `git worktree list --porcelain`, branch refs, `status --porcelain=v2`,
expected and actual `HEAD`, product metadata, process tree, and terminal outcome. Use only synthetic
content and redact absolute user paths.

### Worktree Performance Profile

Amend the existing performance evidence contract with:

- cold/warm repository discovery for 1, 8, 32, and 128 worktrees;
- external-add-to-visible latency;
- create-to-agent-terminal-ready latency;
- switch-to-interactive latency;
- clean and blocked removal latency;
- incremental disk size and full desktop/daemon/agent process-tree RSS/CPU.

Record OS/filesystem, repository object count and status, Git version, terminal geometry,
authentication, agent start policy, run count, raw samples, and exact source/artifact identities.
Until those samples exist, “Orca feels faster” and “native must be faster” are both unverified.

## Recommended Order

This is the dependency order only for Orca-derived work. It does not move these P1/P2 items ahead
of the existing portfolio P0s, and a lane starts only when its files and owners are disjoint.

1. Ship the P1 authoritative read model with explicit bounds, fixtures, and no mutation.
2. Ship the P1 TUI portfolio and a real-terminal external-change capture.
3. Add the P1 Admin read DTO/API with authentication, redaction, and no-mutation tests.
4. Add optional activity, session, source, PR, and delivery links only after each existing owning
   projection has landed; unavailable remains a valid value meanwhile.
5. Amend existing session provenance with P2 worktree-scoped start/resume only if operators use the
   portfolio as a frequent navigation surface.
6. Run the shared lifecycle and performance experiments under their existing P0 owner.
7. Consider manual import/archive/removal only after observed use demonstrates a need and the
   separate safety contract is reviewable.

Every implementation row is one branch/worktree/PR, lands through `prerelease`, and leaves the
integration checkout and disposable worktree clean.

## Success Audit

This analysis is acted on successfully when:

- Akra lists registered worktrees up to explicit bounds without mutating Git or the pool, and marks
  truncation non-authoritative instead of silently omitting it;
- every row declares registration authority, status freshness, managed binding, and ownership
  separately rather than guessing from a path or one boolean;
- external add/remove and locked/dirty states reconcile without restart; removed external rows
  disappear without an invented tombstone, while exact authority-backed missing managed rows remain
  blocked and visible;
- managed rows link to the exact lease generation and only show task, official session, source, PR,
  and delivery fields supplied by their existing owning projections; unavailable is explicit;
- the TUI and Admin consume one application projection and inbound adapters contain no worktree
  policy;
- an external or ambiguous path never becomes a lane or deletion target implicitly;
- fixed slot capacity, reviewed frozen delivery, and conservative cleanup remain unchanged;
- permission and sandbox bypass never derive from worktree presence;
- any performance claim includes the shared raw process-tree and Git-state evidence contract;
- a future destructive action proves target, ownership, dirtiness, locks, expected branch `HEAD`,
  runtime-resource scope, and terminal reconciliation before reporting success.
