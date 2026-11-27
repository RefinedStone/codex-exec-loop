# CI acceptance receipts: November 27, 2026

`acceptance.complete=false`. This supplements the [original acceptance report](ci-rollout-acceptance-20261127.md) at `prerelease@865e0ade186221b24d02a4f54ab58d53f78b693d`. The report remains the source for the pre-extraction baseline and original inventory. This receipt does not modify that report, the gates, or postmerge behavior.

## Reproduce the local receipts

Use a disposable clean checkout at the SHA above, Node 24, and the pinned Rust 1.95.0 toolchain. Offline Cargo commands assume the locked dependencies are available. The local results below used Windows x64 with PowerShell and Linux x64 with Bash under WSL. Use a fresh target for independent compilation claims. Do not reuse a target across operating systems.

Run the package tests on either host:

```text
cargo test --locked --offline --all-features --no-fail-fast -p akra-issue-core -p akra-native -p akra-verification -p akra-verification-native
node scripts/check_crate_boundaries.mjs
```

On Windows, a separate fresh-target run of `cargo test --locked --offline --all-features -p akra-issue-core --verbose` passed 53 unit tests and 2 doctests. The target contained `akra_issue_core` artifacts and no root test executable or library. The ownership check accepted every workspace member and dependency kind.

To reproduce stale-plan rejection, run this PowerShell example in a new disposable checkout. Do not run it in a checkout with existing changes or with the probe file already present. The Windows wrapper must exit 1 before running a gate command. The final command prints the plan for a real untracked member input; it does not run the full gate.

```powershell
New-Item -ItemType Directory -Force artifacts/ci-acceptance | Out-Null
node scripts/agent-plan.mjs --base HEAD --json |
    Set-Content -Encoding UTF8 artifacts/ci-acceptance/before.json
New-Item -ItemType File crates/issue-core/src/ci_acceptance_probe.rs | Out-Null
try {
    powershell -NoProfile -File scripts/check_native_pr.ps1 -Mode Changed `
        -PlanFile artifacts/ci-acceptance/before.json
    Write-Output "Wrapper exit: $LASTEXITCODE"
    node scripts/agent-plan.mjs --base HEAD --explain
} finally {
    Remove-Item -LiteralPath crates/issue-core/src/ci_acceptance_probe.rs
}
```

The recorded stale-plan check used an untracked Markdown file in `docs/validation/`. A separate untracked `.rs` probe in `crates/issue-core/src/` produced `root-consumer-requires-full` and retained all five packages. For the current policy-suite receipt, copy the `node --test` command from the `Test CI and agent planning policy` step in `.github/workflows/native-pr-checks.yml` at the recorded SHA: **201 passed, 0 failed, 0 skipped**. Its cases include dirty index/worktree inputs and invalid result rows. Fixture results do not demonstrate real selected Actions execution.

## Current-source member outcomes

| Package | Windows units: pass/fail/ignored | Linux WSL units: pass/fail/ignored | Doctests passed on each host |
| --- | --- | --- | --- |
| `akra-issue-core` | 53/0/0 | 53/0/0 | 2 |
| `akra-native` | 39/3/1 | 40/6/1 | 1 |
| `akra-verification` | 24/0/0 | 24/0/0 | 0 |
| `akra-verification-native` | 23/0/0 | 11/11/0 | 0 |

Both multi-package commands exited 101. Windows failures were `trusted_executable::fingerprint::tests::{a_modified_pinned_executable_cannot_keep_its_prepared_identity,pinned_native_format_and_digest_describe_the_same_bytes}` and `trusted_executable::tests::native_host_git_and_node_formats_are_accepted`. The failures report mutation rights for another principal on an executable ancestor ACL. No permission or trust policy was changed.

The Linux WSL root volume was full. The reported Linux row uses a temporary writable Linux memory filesystem for home and temp, and a separate target on a volume with space. Native diagnostics still included out-of-space errors and rejection of a world-writable executable ancestor. The Linux runs are not a green native or adapter result, and do not establish a source regression or replace hosted results.

## Hosted receipts and exact rows still open

Download immutable plan and performance artifacts by run ID; the existing Actions retention policy applies:

```text
gh run download 45989408220 -R RefinedStone/codex-exec-loop -n ci-validation-plan-45989408220-1
gh run download 46055046551 -R RefinedStone/codex-exec-loop -n ci-validation-plan-46055046551-1
gh run download 46055992740 -R RefinedStone/codex-exec-loop -n ci-performance-rust_tests-Linux-X64
```

| Issue | Evidence added or refreshed | Unverified acceptance rows |
| --- | --- | --- |
| [#2903](https://github.com/RefinedStone/codex-exec-loop/issues/2903) | [Merged extraction gate](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45988928891/job/140496796910), current Windows isolated build and ownership check. | Final-source baseline-to-head name/ignore parity on Linux and macOS; all-feature and non-lib inventories, including root binary, integration, and doctest coverage. |
| [#2906](https://github.com/RefinedStone/codex-exec-loop/issues/2906) | Real Windows stale-plan rejection, member fallback and both-host package attempts. | Real POSIX wrapper and Windows/POSIX argv parity on the same dirty checkout; real staged, unstaged, deletion, and rename cases beyond the policy fixtures; selected versus full execution parity. |
| [#2907](https://github.com/RefinedStone/codex-exec-loop/issues/2907) | [Planner PR run](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45989408220) was full for global CI inputs. [Later PR run](https://github.com/RefinedStone/codex-exec-loop/actions/runs/46055046551) was full for the root consumer. Both `matrix.json` files had zero selected rows. | A real selected package/target matrix on Linux, Windows and macOS for default and all features, plus a failed-row-only retry on the same SHA and plan. The real workspace has a root consumer for each current non-root member. Neither the empty full matrix nor a fixture substitutes for these rows. |
| [#2908](https://github.com/RefinedStone/codex-exec-loop/issues/2908) | [Native extraction predecessor](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45989193073) passed macOS and Linux Rust Tests. Its Windows job exceeded the one-hour limit. | Current-source green Windows ACL and Linux native group execution; current-source macOS native contracts; cross-platform moved/retained inventory parity. |
| [#2909](https://github.com/RefinedStone/codex-exec-loop/issues/2909) | [Later macOS predecessor job](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45989233028/job/140485512210) passed; its aggregate gate failed. Independent verification unit tests passed on both local hosts. | Final matrix on one SHA, including root binding-generation, lease/authority, restart, and serialization contracts. |
| [#2910](https://github.com/RefinedStone/codex-exec-loop/issues/2910) | [Adapter extraction predecessor](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45989271718) passed all three OS Rust test jobs but failed lint and the aggregate gate. Current standalone Windows adapter passed 23 tests. | Final-source green Linux and macOS adapter results, moved/retained plus ignored/helper inventory, and matched repeated timings for the four expensive groups. |
| [#2864](https://github.com/RefinedStone/codex-exec-loop/issues/2864) | [Passing Windows portable predecessor](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45989154295/job/140472743486) reported 2783.94 s for the step. The #2910 predecessor reported 2719.95 s on the same image version but a different SHA and scope. | Matched Windows before/after repeats with unchanged inventory: per-filter timings, timeout variance, and full job median. |
| [#2865](https://github.com/RefinedStone/codex-exec-loop/issues/2865) | The [Linux baseline](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45936932223/job/140302241613) and [candidate](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45953138161/job/140353445633) used different image versions (`20261122.9.1` and `20261116.202.1`). The original single samples showed a faster architecture phase and a slower root-lib phase. | Matched repeated Linux samples, per-test root-lib and architecture distributions with unchanged inventory, and a Rust Tests job median. |
| [#2866](https://github.com/RefinedStone/codex-exec-loop/issues/2866) | [Trusted Linux job](https://github.com/RefinedStone/codex-exec-loop/actions/runs/46055992740/job/140698668642) restored a fallback target, matched a sealed snapshot, reused timestamps for 3,700 of 3,749 tracked files, then sealed and saved. `rust-tests`: 336 Fresh, 1 Compiling. The Dirty paths reported by `rust-tests` and `native-confirmation` both changed between cache source `040d624c1953b7c0dab4640ad8e17cad5a0b6b45` and run source `15fba7f4be6392ec8c2e3f2b3c910a194bd4b434`. | Trusted warm no-Rust versus relevant-edit pairs on Linux, Windows and macOS; selected cache namespace warm coverage. The hosted observation supports actual Linux changed-input behavior only. |
| [#2911](https://github.com/RefinedStone/codex-exec-loop/issues/2911), [#2690](https://github.com/RefinedStone/codex-exec-loop/issues/2690) | Latest immutable-source evidence is separated from older runs; local and job timings do not imply an overall speedup. | Matched cold/warm samples, queue and gate wall time, runner occupancy, preserved inventory, complete green results and full/selected parity. Postmerge stays full. |
