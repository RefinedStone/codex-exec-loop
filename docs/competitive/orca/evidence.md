# Orca Evidence Ledger

[한국어 번역](../../ko/competitive/orca/evidence.md)

This ledger separates immutable source inspection, installed-artifact observation, documented
behavior, inference, and unverified experiments. Product conclusions are in
[analysis.md](analysis.md), and Akra work decisions are in [gap-matrix.md](gap-matrix.md).

## Snapshot

| Field | Value |
| --- | --- |
| Product | StablyAI Orca |
| Official repository | <https://github.com/stablyai/orca> |
| License | MIT |
| Stable release | [v1.4.137](https://github.com/stablyai/orca/releases/tag/v1.4.137) |
| Release source commit | `6013055491943336660e12e5dec93c9ece4575bb` |
| Source commit date | 2026-07-12 04:09:01 +00:00 |
| Release publication | 2026-07-12 04:40:30 +00:00 |
| Latest prerelease observed | v1.4.138-rc.7, 2026-07-13 |
| Audit date | 2026-07-14 (Asia/Seoul) |
| Akra baseline | `6274a7fc9703f85e4fb6247541dc0d4ed6c5fb9b` on `prerelease` |
| Akra version | 1.3.5 |
| Audit environment | Ubuntu 24.04.2 under WSL2, Linux 6.18.33.2, x86_64, Git 2.43.0 |

The public source was cloned read-only and checked out detached at the release tag. The installed
Windows application independently identified itself as StablyAI Orca v1.4.137 and pointed its
updater at `stablyai/orca`.

Installed artifact identity:

| Artifact | Metadata | SHA-256 |
| --- | --- | --- |
| `Orca.exe` | Product `Orca`, company `stablyai`, product version `1.4.137.0`, file version `1.4.137` | `40dbd16873223a0c983c7f0453cd09943c44e0169dec43be0fd4c99c1e68a91f` |
| `resources/app.asar` | package version `1.4.137`, author `stablyai`, homepage `https://github.com/stablyai/orca` | `4bb73a15d8426219df6aa5da80c4e01916393f01804c67d0d982a4739ee28f5e` |

The installed updater configuration named GitHub owner `stablyai`, repository `orca`, and release
channel `release`. No username-specific installation path is retained in this ledger.

## Evidence Classes

- `verified`: inspected at the immutable source commit, observed in installed artifact metadata, or
  reproduced locally;
- `documented`: explicitly stated by official Orca documentation but not reproduced;
- `proposed`: labeled experimental, beta, or otherwise not a stable default contract;
- `inferred`: a bounded consequence reasoned from verified facts without an end-to-end reproduction;
- `unverified`: missing the required environment, raw sample, dependency install, authenticated
  provider, or external-policy evidence.

Source presence proves an implementation path, not that every packaged configuration exercised it.
Tests read during a static audit are quality intent, not green release results. Inventory counts and
file size are not runtime performance evidence.

## Audit Environment And Privacy

The Windows desktop app was running while its packaged CLI was called from WSL. `orca status
--json` reported the application, runtime, and graph ready. The package has no useful CLI version
subcommand; release identity therefore comes from Windows file metadata, packaged application
metadata, updater configuration, release metadata, and the pinned source tag rather than help text.

The CLI's fleet commands can return private repository names, absolute paths, agent prompts,
terminal previews, runtime IDs, and pane handles. Those raw responses were inspected only to verify
behavior and were not committed, quoted, or retained as a capture. This ledger records only the
following sanitized observation for the public Akra repository:

- `git worktree list` and Orca each reported eight worktrees;
- the set was one primary checkout, three detached Akra pool slots, and four branch worktrees;
- the audit worktree had been created outside Orca and appeared without restarting Orca;
- the repository policy allowed external worktrees to remain visible.

No private repository, home-directory path, prompt, terminal contents, repository UUID, runtime ID,
or user identity is evidence for a product conclusion here.

## Reproduction Commands

### Installed Identity

Run from PowerShell on the Windows host:

```powershell
$root = Join-Path $env:LOCALAPPDATA 'Programs\orca'
(Get-Item (Join-Path $root 'Orca.exe')).VersionInfo |
  Select-Object ProductName, ProductVersion, FileVersion, CompanyName
Get-FileHash -Algorithm SHA256 (Join-Path $root 'Orca.exe')
Get-FileHash -Algorithm SHA256 (Join-Path $root 'resources\app.asar')
Get-Content (Join-Path $root 'resources\app-update.yml')
```

The packaged `app.asar` metadata was inspected locally for `name`, `version`, `author`, and
`homepage`. The updater and package identity matched the public repository.

### Source And Release Identity

```bash
git clone https://github.com/stablyai/orca.git /tmp/orca-v1.4.137-source
git -C /tmp/orca-v1.4.137-source checkout --detach v1.4.137
git -C /tmp/orca-v1.4.137-source rev-parse HEAD
git -C /tmp/orca-v1.4.137-source show -s --format='%H%n%cI%n%s' HEAD
gh api repos/stablyai/orca/releases/tags/v1.4.137 \
  --jq '{tag_name,published_at,draft,prerelease,target_commitish}'
```

The annotated tag was peeled to
`6013055491943336660e12e5dec93c9ece4575bb` before source conclusions were recorded.

### Runtime And Worktree Inventory

The exact Windows path can be resolved from `$env:LOCALAPPDATA`; the examples below intentionally
avoid a user-specific path:

```powershell
$cli = Join-Path $env:LOCALAPPDATA 'Programs\orca\resources\bin\orca.cmd'
& $cli status --json
& $cli worktree ps --json
& $cli repo show --repo 'id:<repo-id>' --json
& $cli worktree list --repo 'id:<repo-id>' --limit 100 --json
```

Before sharing any output, remove repository IDs, absolute paths, prompts, terminal previews,
runtime IDs, pane handles, and non-public repository names. The audit compared only the public Akra
repository subset with:

```bash
git worktree list --porcelain
git branch -vv
git status --porcelain=v2 --branch
```

### Akra Baseline

```bash
git fetch origin
git worktree add -b docs/native-platform-analysis-orca-worktrees \
  ../codex-exec-loop-worktrees/docs-native-platform-analysis-orca-worktrees \
  origin/prerelease
git -C ../codex-exec-loop-worktrees/docs-native-platform-analysis-orca-worktrees rev-parse HEAD
```

The baseline was clean when the audit worktree was created. The audit read Akra core, application
services and ports, TUI and Admin inbound adapters, app-server and Git/GitHub outbound adapters,
parallel-mode implementation, current operator contract, and control-plane architecture.

## Product Release And Topology

| Claim | Class | Evidence |
| --- | --- | --- |
| The product is StablyAI Orca, not another similarly named terminal product. | verified | installed metadata, updater target, [package metadata](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/package.json) |
| v1.4.137 is an Electron desktop ADE with an embedded terminal and packaged CLI. | verified | installed artifact layout, [package metadata](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/package.json), [install docs](https://www.onorca.dev/docs/install) |
| The app coordinates local, WSL, and SSH hosts. | verified/documented | host-provider and relay source, [SSH docs](https://www.onorca.dev/docs/ssh) |
| The packaged runtime was ready during the audit. | verified | sanitized `orca status --json` observation |
| Mobile, automations, hibernation, and orchestration extend the desktop loop. | documented/proposed | [mobile](https://www.onorca.dev/docs/mobile), [automations](https://www.onorca.dev/docs/cli/automations), [hibernation](https://www.onorca.dev/docs/agents/hibernation), [orchestration](https://www.onorca.dev/docs/cli/orchestration) |

## Worktree Discovery And Ownership

The following immutable files were inspected:

- [`src/main/git/worktree.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/git/worktree.ts): Git worktree list parsing,
  cache/singleflight, mutation invalidation, sparse inspection, add, and remove primitives;
- [`src/main/ipc/worktrees.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/ipc/worktrees.ts): list/detect/create/remove IPC and
  reconciliation;
- [`src/main/ipc/worktree-logic.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/ipc/worktree-logic.ts): name sanitization, generated path,
  WSL/workspace layout, and safety helpers;
- [`src/shared/worktree-ownership.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/shared/worktree-ownership.ts): managed, external, and legacy
  ownership classification and visibility;
- [`src/shared/types.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/shared/types.ts): worktree metadata, source, links, and lineage;
- [`src/cli/handlers/worktree.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/cli/handlers/worktree.ts): CLI selectors and worktree operations.

Verified findings:

- fresh discovery begins with Git's worktree inventory and parses path, `HEAD`, branch, detached,
  bare, and lock state; sparse-checkout state is added by separate bounded probes;
- NUL-delimited porcelain is preferred, with compatibility handling for older Git behavior;
- concurrent scans share one in-flight operation and mutation generation prevents stale reuse;
- sparse-checkout inspection uses bounded concurrency;
- ownership distinguishes managed, external, and unknown legacy state rather than trusting a path;
- local external mutations are reconciled into the product view;
- a disconnected remote host can return metadata fallback explicitly marked non-authoritative.

The installed runtime observation independently verified external discovery for the audit worktree.
It did not test every ownership transition or remote fallback.

## Creation, Bootstrap, And Metadata

Creation paths were inspected in:

- [`src/main/git/worktree.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/git/worktree.ts);
- [`src/main/ipc/worktrees.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/ipc/worktrees.ts);
- [`src/main/ipc/worktree-remote.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/ipc/worktree-remote.ts).

Verified findings:

- base selection accepts local refs, remote refs, and commit identities, with refresh and offline
  fallback behavior;
- new branches use `git worktree add --no-track -b <branch> <path> <base>`;
- the actual base is recorded in branch configuration;
- `push.autoSetupRemote=true` is written only when the user has no explicit value;
- collision handling, existing-branch preservation, pull-request start points, sparse setup,
  rollback, and local/SSH host routes are separate cases;
- worktree metadata can retain source, base, push target, issue/PR links, agent, prior IDs, and
  parent/child lineage;
- setup is read from the created worktree and run visibly in a terminal;
- local creation has a bounded timeout for known filesystem-stall cases.

Official worktree creation, background progress, cancellation/retry, import, and lifecycle behavior
is documented in [Worktrees](https://www.onorca.dev/docs/model/worktrees). The audit did not create
or delete a disposable Orca-managed worktree because the live Orca installation contained active
user workspaces and no destructive product mutation was necessary to establish the source design.

### Base And Drift Reconciliation

[`src/main/runtime/orca-runtime.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/runtime/orca-runtime.ts#L16035-L16240)
contains the current base-status reconciliation and drift probe.

- after creation, a current optimistic token guards the remote fetch result so stale reconciliation
  cannot overwrite a newer worktree state;
- the created base SHA is compared with the refreshed remote-tracking ref and projected as
  `current`, `drift`, `base_changed`, or `unknown`, with bounded behind count and recent subjects;
- an on-demand drift probe resolves stored/default base metadata, fetches best effort, and reports
  how far `HEAD` is behind;
- the probe returns unknown when it cannot establish a signal. SSH repositories are explicitly
  unknown because the local helper does not implement equivalent remote-ref/log plumbing;
- unknown does not become an immutable delivery gate. This is interactive drift information, not
  Akra-style frozen-base authority.

## Terminal, Session, And Remote Continuity

Immutable implementation evidence:

- [`src/main/runtime/orca-runtime.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/runtime/orca-runtime.ts): worktree-scoped terminal
  lookup/create/stop, background runtime, worktree lifecycle composition;
- [`src/main/persistence.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/persistence.ts): PTY binding, serialized flush,
  generation guard, atomic replacement, rotating backups, recovery, and host separation;
- [`src/main/providers/local-pty-provider.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/providers/local-pty-provider.ts) and
  [`src/main/pty/wsl-orca-env.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/pty/wsl-orca-env.ts): worktree and pane/tab/terminal
  handles in PTY environments;
- [`src/main/providers/ssh-git-provider.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/providers/ssh-git-provider.ts) and remote relay
  handlers: remote worktree Git operations and reconnection;
- [`src/shared/ssh-types.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/shared/ssh-types.ts): relay disconnect-policy bounds and
  defaults.

Verified source findings:

- terminals and saved layout are keyed to worktree and execution host;
- stable identifiers are allocated before PTY creation and bindings are synchronously persisted
  before spawn success is returned;
- persistence serializes writes, rejects stale generations, replaces through temporary files, and
  keeps rotating recovery backups;
- persistence partitions local, SSH, and remote-runtime hosts; WSL shares the local session
  partition while its Git/path/cache/environment routing remains explicit;
- SSH providers perform actual worktree operations on the remote host rather than mirroring a local
  directory fiction;
- an established SSH relay follows a configurable disconnect policy; the audited default is “until
  reset,” while bounded values range from 60 seconds through seven days. The current official
  [SSH page](https://www.onorca.dev/docs/ssh) instead states a five-minute default; tag-source facts
  take precedence for this v1.4.137 snapshot and the mismatch remains explicit.

Official docs distinguish [session restore](https://www.onorca.dev/docs/model/session-restore) from
experimental [hibernation](https://www.onorca.dev/docs/agents/hibernation). App-close PTY survival,
machine-reboot restoration, and agent conversation resume are not treated as one guarantee here.
No forced process crash, host reboot, or SSH disconnect was reproduced during this audit.

## Removal And Recovery

Immutable implementation evidence:

- [`src/main/worktree-removal-safety.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/worktree-removal-safety.ts): dangerous-target,
  nested-worktree, `.git` backlink, admin-entry, symlink, and ownership proof;
- [`src/main/local-worktree-removal-recovery.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/local-worktree-removal-recovery.ts): Git for Windows
  partial-removal recovery;
- [`src/main/git/worktree.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/git/worktree.ts): worktree removal, safe branch
  deletion, preservation, and prune retry;
- [`src/main/ipc/worktrees.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/ipc/worktrees.ts) and
  [`src/main/runtime/orca-runtime.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/runtime/orca-runtime.ts): full removal ordering and
  runtime-resource cleanup.

Verified findings:

- concurrent duplicate removal is coalesced;
- registered targets are re-resolved from canonical Git inventory; Orca ownership is not required
  for every registered external worktree;
- primary, dangerous, nested, and locked targets are refused, while an unregistered/orphan recursive
  recovery additionally requires structural `.git` and Orca provenance proof;
- non-force dirty/untracked preflight occurs before worktree PTY teardown;
- only target-owned PTYs/watchers are asked to stop, best effort; close errors are logged and removal
  may continue;
- normal branch cleanup uses `git branch -d`, preserving unmerged or unpublished history;
- force applies to worktree removal; a separate later deletion of a preserved branch can compare the
  previously observed `HEAD` with `git update-ref -d`;
- moved branches and branches checked out elsewhere are preserved;
- stale registration, orphan paths, already-removed worktrees, and Windows partial deletion have
  distinct recovery paths;
- authoritative refresh removes missing entries from the live view, prunes missing child lineage,
  and may rotate missing-parent instance identity, but does not immediately erase every durable
  `worktreeMeta` entry.

Representative static test evidence:

- [`src/main/git/worktree.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/git/worktree.test.ts);
- [`src/main/worktree-removal-safety.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/worktree-removal-safety.test.ts);
- [`src/main/local-worktree-removal-recovery-live.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/local-worktree-removal-recovery-live.test.ts);
- [`src/main/ipc/worktrees.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/ipc/worktrees.test.ts);
- [`tests/e2e/worktree-lifecycle.spec.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/tests/e2e/worktree-lifecycle.spec.ts).

These tests were read, not executed in the dependency-free source checkout.

## Git, Hosted Review, And Delivery

Immutable implementation evidence:

- [`src/main/source-control/hosted-review-creation.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/source-control/hosted-review-creation.ts): preflight and
  immediate pre-create validation;
- [`src/main/github/pr-start-point.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/github/pr-start-point.ts): fetched and verified PR/fork
  start points;
- [`src/main/github/pr-refresh-coordinator.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/github/pr-refresh-coordinator.ts): linked/branch refresh
  coalescing, host scope, visibility, rate budget, backoff, and post-push delay;
- [`src/main/github/client.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/github/client.ts): PR creation/reconciliation, checks,
  review, comments, and merge;
- [diff viewer](https://www.onorca.dev/docs/review/diff-viewer),
  [commit and push](https://www.onorca.dev/docs/review/commit-push), and
  [hosted review](https://www.onorca.dev/docs/review/github) docs.

Verified findings:

- creation checks branch/dirty/upstream/ahead-behind/auth/existing-review state and rechecks before
  the side effect;
- fork start points fetch and verify the remote SHA;
- draft/template creation and ambiguous-create reconciliation are supported;
- normal push does not silently force, and force is explicit and lease-aware;
- merge blockers are checked and provider-side automatic branch deletion is avoided;
- terminal-observed PR URLs require branch association before linking.

Unverified as one Orca authority contract:

- pre-worker freeze of credential-redacted remote URL, repository identity/visibility, integration
  branch and OID;
- exact reviewed source SHA and ordered commit-range authority;
- review/check/mergeability as a reviewed-by-default gate with explicit bypass provenance;
- serialized detached integration, remote integration-ref verification, and frozen-SHA cleanup.

The audit found separate capable implementation paths, not evidence of that complete Akra-style
chain. Absence from this audit is not a claim that no optional automation can approximate parts of
it.

## Automation, Permissions, And Telemetry

| Claim | Class | Evidence |
| --- | --- | --- |
| Worktree selectors, JSON output, comments/checkpoints, and creation can be driven through the CLI. | verified/documented | CLI source, [CLI overview](https://www.onorca.dev/docs/cli/overview), [checkpoints](https://www.onorca.dev/docs/cli/worktree-checkpoints) |
| Persistent task/message/dispatch/decision-gate orchestration exists but is experimental. | proposed | source and [orchestration docs](https://www.onorca.dev/docs/cli/orchestration) |
| Scheduled automation can target repositories or existing worktrees. | documented | [automation docs](https://www.onorca.dev/docs/cli/automations) |
| New supported-agent launches prefill high-autonomy approval/sandbox bypass arguments; global Manual mode and agent overrides can change them. | documented/source-verified | [supported agents](https://www.onorca.dev/docs/agents/supported), agent launch and persistence source |
| A worktree does not isolate OS processes, credentials, networking, Git config, hooks, filters, or services. | verified/inferred | Git worktree semantics plus inspected shared-config/setup paths |
| What Orca documents as anonymous product-usage telemetry includes fixed enums, version strings, and a local random ID, uses PostHog US, and can be disabled; prompts, file contents, terminal output, and path names are documented as excluded. PostHog plan-level default retention applies and no Orca custom window is documented. | documented | [telemetry docs](https://www.onorca.dev/docs/telemetry) |

No packet capture or third-party agent-provider audit was run. Telemetry conclusions are therefore
documentation claims, not independent network verification.

## Quality Evidence

The static audit found focused tests for:

- concurrent scan sharing, mutation generations, NUL/newline paths, old Git fallback, and bounded
  creation stalls;
- collision suffixes, PR SHA creation, sparse setup, WSL routing, SSH creation, and invalidation;
- forged ownership, nested worktrees, dangerous roots, symlinks, and separate Git directories;
- live Git for Windows partial removal recovery;
- atomic persistence, stale-write rejection, PTY bindings, backup rotation, corruption recovery,
  and host-scoped session separation;
- fork PR start points, refresh isolation/backoff, and pre-create revalidation;
- worktree-scoped tabs, files, browsers, state restoration, and terminal isolation in E2E specs.

Representative immutable tests outside the removal ledger include:

- [`src/main/persistence.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/persistence.test.ts);
- [`src/main/providers/ssh-git-provider.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/providers/ssh-git-provider.test.ts);
- [`src/relay/git-handler-worktree-ops.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/relay/git-handler-worktree-ops.test.ts);
- [`src/main/github/pr-start-point.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/github/pr-start-point.test.ts);
- [`src/main/github/pr-refresh-coordinator.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/github/pr-refresh-coordinator.test.ts);
- [`src/main/source-control/hosted-review-creation.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/source-control/hosted-review-creation.test.ts).

The source checkout intentionally had no `node_modules`; no Orca unit, integration, E2E, package,
signing, or release workflow was executed. The installed artifact was not modified. No release gate
is claimed green from static test presence.

## Akra Baseline Evidence

The Akra comparison read these current boundaries at
`6274a7fc9703f85e4fb6247541dc0d4ed6c5fb9b`:

- [`src/core/`](../../../src/core): headless app command/effect/snapshot runtime;
- [`src/application/service/`](../../../src/application/service) and
  [`src/application/port/`](../../../src/application/port): application ownership and outbound
  boundaries;
- [`src/adapter/inbound/tui/`](../../../src/adapter/inbound/tui) and
  [`src/adapter/inbound/admin_api/`](../../../src/adapter/inbound/admin_api): current operator
  projections;
- [`src/adapter/outbound/app_server/`](../../../src/adapter/outbound/app_server): official Codex
  runtime boundary;
- [`app_server/mod.rs`](../../../src/adapter/outbound/app_server/mod.rs),
  [`execution_policy.rs`](../../../src/adapter/outbound/app_server/execution_policy.rs), and
  [`connection.rs`](../../../src/adapter/outbound/app_server/connection.rs): isolated worker child,
  exact workspace/trust validation, execution policy, and unattended approval decline;
- [`src/application/service/parallel_mode/`](../../../src/application/service/parallel_mode):
  pool, slot lifecycle, completion, orchestration, distributor, and control plane;
- [`pool/reconcile.rs`](../../../src/application/service/parallel_mode/pool/reconcile.rs): fixed
  detached-slot provisioning and conservative reconciliation;
- [`pool/cleanup.rs`](../../../src/application/service/parallel_mode/pool/cleanup.rs): path,
  ownership, lease, clean/integrated, and cleanup identity checks;
- [`slot_lifecycle.rs`](../../../src/application/service/parallel_mode/slot_lifecycle.rs): frozen
  target acquisition and generation-bound lease transitions;
- [`parallel_worker_commit.rs`](../../../src/adapter/outbound/git/parallel_worker_commit.rs):
  exact-root, branch, `HEAD`, base, filesystem, staging, and clean-result validation;
- [`distributor/`](../../../src/application/service/parallel_mode/distributor): serialized source,
  PR, integration, verification, and cleanup delivery;
- [`github/automation.rs`](../../../src/adapter/outbound/github/automation.rs): isolated frozen
  Git/GitHub delivery target and compare-if-unchanged remote operations;
- [`current-product.md`](../../reference/current-product.md) and
  [`architecture.md`](../../reference/architecture.md):
  shipped operator and architecture contracts.

Verified Akra baseline findings:

- parallel automation owns exactly three generated slot worktrees, provisioned as detached
  baselines while idle, plus a generated detached integration worktree; it does not own every
  developer worktree in the repository;
- each parallel worker starts a separate unattended app-server child at the lease worktree's exact
  absolute cwd; bootstrap is read-only, the turn uses the shared workspace-write execution policy,
  ancestor trust is forced untrusted, the applied cwd is revalidated, and approval requests decline;
- a repository-scoped OS mutation lock and SQLite lease authority protect pool mutation;
- every new slot allocation has a 64-lowercase-hex CSPRNG generation propagated through lifecycle,
  completion, delivery, and cleanup CAS checks;
- lease acquisition freezes the push remote, credential-redacted canonical GitHub URL, repository
  identity/visibility, integration branch, and remote base OID before worker start;
- the worker contract asks agents to leave edits uncommitted; a host-owned bounded path validates
  exact workspace, branch, base, `HEAD`, index/filesystem constraints, and clean output before
  commit readiness, while a clean existing linear descendant commit remains an explicit
  compatibility path;
- reviewed delivery defaults to PR approval, checks, clean mergeability, and matching frozen PR head,
  with explicit parent-level high-risk exceptions;
- integration is serialized in a dedicated detached worktree and remote state is verified before
  identity-checked branch/slot cleanup;
- `:parallel off` stops automation but intentionally does not delete worktrees;
- the TUI board and authenticated Admin dashboard project the managed pool, roster, lifecycle,
  queue, distributor, and delivery state, but there is no repository-wide managed/external
  worktree portfolio or general worktree jump.

## Audit Limits

- The installed Orca CLI was used read-only for identity and status/inventory observation. No live
  user worktree, branch, terminal, prompt, repository setting, pull request, or automation was
  mutated.
- A disposable end-to-end create/setup/dirty/lock/remove/orphan/recovery sequence was not executed.
  Those conclusions are immutable-source verified and selectively supported by static tests, not
  dynamically reproduced in the installed app.
- No SSH host, WSL Git mutation, mobile device, hibernated agent, scheduled automation, or
  orchestration coordinator was exercised.
- No authenticated GitHub/GitLab PR creation, review, merge, force-push, or branch retirement was
  executed.
- No telemetry packet capture, secret canary, permission-bypass canary, repository-hook attack, or
  malicious setup file was executed.
- No Orca tests were run because dependencies were absent from the read-only source checkout.
- No complete process-tree performance benchmark was captured. Startup, render, memory, worktree
  creation, switching, removal, and disk-growth winners remain unknown.
- The latest prerelease was recorded only for freshness; all behavior conclusions are bound to the
  stable v1.4.137 source commit.
