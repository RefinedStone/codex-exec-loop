# Trusted warm-cache evidence for #2866

**`acceptance.complete=false`.** Five complete Linux jobs and the build phases of one Windows job recorded zero recompiles. The Windows job later failed a contract test. This adds warm-cache evidence to the [acceptance receipts](ci-acceptance-receipts-20261127.md); it does not repeat the [matched test timings](ci-matched-2864-2865-20261128.md) or change cache policy.

## Compare against the restored source

The audit checked 36 lane artifacts from 18 trusted `prerelease` pushes between runs `46055992740` and `46058516076`. One cancelled Windows lane had no completed command counters. For each warm sample below, the source suffix of the restored target `matchedKey` equals `previousRevision` in `cache-inputs-prepare.json`. Compare that cached revision against the checked-out `sourceRevision`, not against the Git parent of the push. A registry hit or a successful target restore does not by itself demonstrate compilation reuse.

The samples used an exact registry hit and fallback target restore. Both lanes used Rust 1.95.0 (`59807616e`), x64, and the `dev/test` profile:

| Lane | Target and features | Sealed input boundary SHA-256 |
| --- | --- | --- |
| Linux `rust-tests` | Implicit `x86_64-unknown-linux-gnu`, default features | `b2223672b1fcd14f622159d7011e96a0662feedcca96a9076fe631ecf8964e5b` |
| Windows `portable` | Explicit `x86_64-pc-windows-msvc`, all features | `1fc4b767a64a595a8d4f029d3854b344905f8ede46d0e31b47088cba3809e698` |

## Linux: five complete warm jobs without Rust changes

Every row recorded 337 Fresh units in `rust-tests` and 666 in `native-confirmation`, with zero `Compiling`, `Checking`, or `Dirty` units. Both commands exited 0. They still emitted 25 and 1 test-result lines respectively. All five jobs sealed the new snapshot and reported a successful save step.

| Trusted run | Restored source → checked-out source | Changed paths | Restored / tracked | Runner image |
| --- | --- | --- | ---: | --- |
| [46056109635](https://github.com/RefinedStone/codex-exec-loop/actions/runs/46056109635) | `15fba7f4b` → `a8aee6105` | 4 Desktop, 2 docs, 2 npm smoke | 3741 / 3749 | `20261122.9.1` |
| [46056117638](https://github.com/RefinedStone/codex-exec-loop/actions/runs/46056117638) | `a8aee6105` → `9fcf96c6b` | 2 Desktop | 3747 / 3749 | `20261122.9.1` |
| [46056128749](https://github.com/RefinedStone/codex-exec-loop/actions/runs/46056128749) | `9fcf96c6b` → `80b115215` | 4 Desktop | 3746 / 3750 | `20261116.202.1` |
| [46056598412](https://github.com/RefinedStone/codex-exec-loop/actions/runs/46056598412) | `494b1b77d` → `67bc68a06` | 6 Desktop | 3744 / 3750 | `20261116.202.1` |
| [46056656419](https://github.com/RefinedStone/codex-exec-loop/actions/runs/46056656419) | `67bc68a06` → `8b565c780` | 5 Desktop | 3745 / 3750 | `20261122.9.1` |

The [changed-input control](https://github.com/RefinedStone/codex-exec-loop/actions/runs/46056136180) used the same boundary and image as the last row. It restored `80b115215c01bffd629ae7038d3df80036e1dd74` and checked out `494b1b77d39f397994fbe0b2e3ec5a74e15bd63c`. The reported Dirty files, `src/application/service/issue_draft.rs` and `src/application/port/inbound/issue_draft_port.rs`, both changed. Each command compiled `akra-ade` once; the commands recorded 336 and 665 Fresh units and both passed.

## Windows: zero recompiles, but the portable job failed

The [consumer job](https://github.com/RefinedStone/codex-exec-loop/actions/runs/46057475352/job/140765346412) restored `cfe79d32e3cc41337a1d91f2f32889e93d74e463` from a [passing producer](https://github.com/RefinedStone/codex-exec-loop/actions/runs/46057355211/job/140733776772) that sealed and saved that cache. It checked out `40b6b41b0521547111091afe8b501b29be4ffdae`. The five changed files were:

- `desktop/src/main/gitStatus.ts`
- `desktop/src/renderer/conversation/WorkingTreePanel.tsx`
- `desktop/src/renderer/conversation/workingTreeNavigation.ts`
- `desktop/test/gitStatus.test.ts`
- `desktop/test/workingTreeNavigation.test.ts`

Both jobs used image `20261123.321.1`. The consumer restored 3749 of 3754 tracked timestamps, with three changed paths, two new paths, and none skipped. The production check finished in 19.15 s and the test build in 0.59 s. These durations include work other than compilation: Cargo recorded 620 Fresh units and zero `Compiling`, `Checking`, or `Dirty` units. Later, `failed_completion_commit_recovers_from_durable_sent_by_reading_only` failed. The portable command exited 1 and skipped the save.

The producer provides a changed-input comparison. It had restored `3110a9e9cfb3eaa83bf32a06af640e4c207dffba`; seven Rust files changed, including both files Cargo reported Dirty. It recorded one `Checking`, one `Compiling`, and 771 Fresh units. `Checking` is not a second compilation. The Linux lane in the Windows consumer run restored `8b565c7804c498487c30afc51cffa8fdb01c4ca9`, which differed in seven Rust files. That lane is not a matching no-Rust sample.

## Why the docs-only push recompiled

[Run `46058427812`](https://github.com/RefinedStone/codex-exec-loop/actions/runs/46058427812) added four validation-docs files relative to push parent `c68c70d11`. The actual target restores were older:

| Lane | Restored revision → checkout `f78cdf30a4840874990fcb27cd14db3ca60b9352` | Actual diff | Dirty paths confirmed in the diff |
| --- | --- | --- | --- |
| Linux | `782b3f81492a3ec78831a5c40dd42eb220f9c993` | 27 paths, 15 Rust | `src/adapter/inbound/kernel_protocol/next.rs`; `src/adapter/inbound/gateway/context.rs` |
| Windows | `cfe79d32e3cc41337a1d91f2f32889e93d74e463` | 58 paths, 33 Rust | `src/adapter/inbound/kernel_protocol/mod.rs`; `src/application/service/kernel_extensions/mod.rs` |

These are real changed-input rebuilds. They do not prove a defect in warm reuse.

## Reproduce and remaining controls

```bash
gh run download 46057475352 -R RefinedStone/codex-exec-loop -n ci-performance-portable_native-Windows-X64
gh run download 46056656419 -R RefinedStone/codex-exec-loop -n ci-performance-rust_tests-Linux-X64
GIT_MASTER=1 git diff --name-status cfe79d32e3cc41337a1d91f2f32889e93d74e463 40b6b41b0521547111091afe8b501b29be4ffdae
GIT_MASTER=1 git diff --name-status 67bc68a06aa41c07fefd498bcfa3884189e3f1de 8b565c7804c498487c30afc51cffa8fdb01c4ca9
```

Inspect `cache-policy.json`, `cache-snapshots.jsonl`, `cache-inputs-prepare.json`, `cache-inputs-seal.json` when present, and `steps.jsonl`. No actionable cache defect was shown. This audit did not find a passing end-to-end Windows warm no-Rust job, a same-source Windows/Linux warm pair, or matched cold controls. It does not establish selected-lane cache coverage or justify a rollout change.
