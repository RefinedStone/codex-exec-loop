# Fast Worktree (CoW / BTRFS) Deep Dive

Interactive HTML: [../reports/grok-build/fast-worktree.html](../reports/grok-build/fast-worktree.html)

Parent product analysis: [analysis.md](./analysis.md) · Evidence: [evidence.md](./evidence.md)

## Snapshot

| Field | Value |
| --- | --- |
| Subject | Grok Build `xai-fast-worktree` |
| Source repo | https://github.com/xai-org/grok-build |
| Source commit | `b189869b7755d2b482969acf6c92da3ecfeffd36` |
| Crate path | `crates/codegen/xai-fast-worktree` |
| Audit date | 2026-07-16 (Asia/Seoul) |
| Method | Static source audit (no runtime benchmarks) |
| Decision class (Akra) | **Eval** — adapter performance experiment only |

## Thesis

Fast worktree redefines git worktree creation as a **filesystem CoW/snapshot problem**:

1. Register a worktree cheaply (`git worktree add --no-checkout`) or snapshot a volume.
2. Populate the working tree via **reflink/CoW parallel copy**, or skip population with **BTRFS/overlay O(1)** paths.
3. Operate a **pre-created pool** and bring trees current with **dirty-aware `WorktreeSync`**.

It optimizes **create / sync / delete latency**. It does **not** replace Akra’s lease, frozen-base, review, or delivery authority.

## Problem

Plain `git worktree add` with full checkout is dominated by single-threaded working-tree population. On large monorepos that cost becomes the majority of agent isolation startup. Delete cost also scales with file count unless a subvolume delete path exists.

## Strategy ladder

For `CreationMode::Linked` and `Standalone` on Linux:

1. **Overlay-on-FUSE** (special workspace stack) → O(1)
2. **BTRFS subvolume snapshot** → O(1)
3. **Parallel CoW file clone** (`reflink_or_copy`) → O(n files), cheap per file on APFS/Btrfs/XFS
4. Implicit fallbacks / `GitCheckout` when simplicity is preferred

macOS primarily uses path (3) via APFS.

## CoW / reflink

- Implementation: `src/copy/cow.rs` → `reflink_copy::reflink_or_copy`
- Shares data blocks until mutation; falls back to regular copy
- Restores source permissions after reflink (executable bit survival)

## Linked + parallel copy

1. `git worktree add --no-checkout` — metadata only
2. Parallel walk (`ignore::WalkBuilder`) + hash-sharded workers
3. Cap workers (8 on macOS for FD limits, 32 elsewhere)
4. Skip `.git` in walk; intentionally ignore global gitignore and `.git/info/exclude` so personal patterns cannot drop tracked files
5. `PartialWorktreeGuard` reclaims half-built destinations on failure

## BTRFS snapshot

- `btrfs subvolume snapshot <source> <dest>`
- Independent subvolume sharing blocks via FS CoW
- Bind-mounted sources: snapshot inside real btrfs mount, expose user path via **symlink** (namespace-persistent)
- Sandbox without `CAP_SYS_ADMIN`: `BtrfsDelegate` privileged helper
- Mandatory post-snapshot git cleanup: `*.lock`, merge/rebase heads, sequencer dirs, stale `worktrees/` registrations

## Overlay-on-FUSE

- Detect FUSE lower + overlay upper (btrfs)
- New overlay mount sharing lower; upper is snapshot of current upper
- Namespace-local mounts; skip or delegate in private mount namespaces
- Mostly internal/cloud workspace optimization, not a laptop-default path

## Creation and working-tree modes

| CreationMode | Meaning |
| --- | --- |
| `Linked` (default) | Shared object store via git worktree |
| `Standalone` | Independent `.git/`; optional `grok-worktree-source` marker |
| `GitCheckout` | Plain full checkout |

| WorkingTreeMode | Meaning |
| --- | --- |
| `PreserveWorkingTree` | Keep dirty/untracked from source |
| `CleanTracked` | Clean tracked tree |
| `CleanAll` | Reset + clean style (ignored not fully cleaned by default git clean) |

`IgnoredFilesMode`: Skip / Copy / CopyOnly with skip patterns.

## Pool and sync

`WorktreeSync` on acquire:

1. Resolve HEADs (gix)
2. `git reset --hard` if needed
3. `git clean -fd` (skippable)
4. Replay dirty state from precomputed `git status --porcelain=v2 -z` (`SourceDirtyState`)

Porcelain v2 is required for full staged vs worktree `XY` semantics. Status is collected once and reused across multiple syncs. Phase timings are reported for diagnosis. `pool_perf_bench` exercises create/warm/sync/release/cleanup.

## Teardown

Prefer overlay unmount → btrfs subvolume delete (O(1)) → generic worktree remove. Optional SQLite metadata tracks inventory and supports orphan GC.

## Akra implications

| Decision | Scope |
| --- | --- |
| **Eval** | Outbound git adapter performance if pool create/sync is a measured bottleneck |
| **Reject as policy** | Replacing lease, frozen OID, review gates, fail-closed delivery |
| **Differentiate** | Keep SQLite planning + review-delivered parallel pool as product center |

### Experiment checklist (if pursued)

1. Measure current Akra create/sync on a real large repo with environment stamp
2. Compare against `--no-checkout` + parallel reflink and, on Linux, btrfs snapshot
3. Keep application/domain policy untouched
4. Verify partial-failure reclaim vs Akra cleanup contracts
5. Adopt only with proof samples; otherwise leave unevaluated

## Evidence pins

Primary paths under `crates/codegen/xai-fast-worktree/`:

- `src/lib.rs`, `src/api.rs`
- `src/worktree/execute.rs`, `src/worktree/plan.rs`
- `src/copy/cow.rs`, `src/copy/engine.rs`, `src/copy/worker.rs`
- `src/btrfs/snapshot.rs`, `src/overlay/*`
- `src/sync.rs`, `src/bin/pool_perf_bench.rs`

Limits: no latency benchmarks in this audit; comments citing ~1.4s status are source-local, not remeasured here.
