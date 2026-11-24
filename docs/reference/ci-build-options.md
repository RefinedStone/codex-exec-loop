# Native Rust validation build options

Issue: [#2690](https://github.com/RefinedStone/codex-exec-loop/issues/2690), stage 2.

The initial source review used prerelease `70966a27f27429f9134e467c98b7b09a9894e39d`.
The final integration was rechecked against the actual prerelease base
`99238e8792b25312d2f1c6ad3e5a0954f1fe05c3`; the contracts below include its added native checks.
This recheck inspected source changes without running more tests or querying CI.

## Build contracts

| Lane | Features | Target | Profile | Validation |
| --- | --- | --- | --- | --- |
| Linux complete Rust suite | default | runner host | Cargo test default | unit, integration, doctests |
| Linux Clippy | all | runner host | Cargo dev default | all targets |
| Windows portable | all | x86_64-pc-windows-msvc | Cargo defaults | production check + 30 library filters |
| macOS portable | all | aarch64-apple-darwin | Cargo defaults | production check + 12 library filters |
| Linux native confirmation | default | runner host | Cargo defaults | Gateway executable + exactly one ignored helper test |
| macOS native confirmation | all | aarch64-apple-darwin | Cargo defaults | Gateway executable + exactly one ignored helper test |

Default-feature Linux tests and all-feature portable checks serve different purposes. Keep both.
A production `cargo check`, executable build, and library test build also produce different artifacts;
their entire compile times are not removable duplication.

`scripts/run_native_rust_contracts.mjs` prepares the library test executable once with Cargo's
JSON artifact report, then runs each requested filter from that exact executable. It only accepts
the unique `codex_exec_loop_native` artifact with `kind: lib` and `profile.test: true`.
Dependencies and production binary artifacts cannot be selected accidentally. Filter execution
remains sequential; shared process/port/filesystem resources do not gain new concurrency.

```sh
node scripts/run_native_rust_contracts.mjs \
  --all-features --target aarch64-apple-darwin --timings -- \
  'process_liveness::tests::' \
  'trusted_executable::tests::native_' \
  'private_sqlite::tests::' \
  'stable_user_data_root::tests::' \
  'sqlite_issue_automation_store::private_store::tests::' \
  'adapter::outbound::vault_key_store::' \
  'application::service::evidence_vault_unlock::' \
  'task_evidence_artifact::tests::' \
  'sqlite_planning_authority_adapter::tests::task_evidence::' \
  'parallel_worker_commit::tree_snapshot::tests::' \
  'parallel_mode::tests::pool::lease::task_evidence::' \
  'worker_evidence::'
```

The runner also accepts `--features <list>`, `--no-default-features`, `--profile <name>`, and
`--target-dir <path>`. Omitting feature switches keeps Cargo's defaults. It passes the same options
to the one preparation command and inherits `CARGO_TARGET_DIR`, `CARGO_BUILD_TARGET`, toolchain,
`RUSTFLAGS`, and profile environment overrides. It never cleans or shares target directories.
Cargo reports the actual executable location, so custom targets and profiles need no guessed paths.

Every filter must match the compiled inventory and execute at least one passing, non-ignored test.
Empty filters, missing artifacts, compile failures, interrupted tests, and zero-test success fail
closed. The runner does not enable ignored tests or doctests. Those retain their explicit, separate
execution paths. The unchanged PowerShell local validation path continues to use Cargo directly;
the Git Bash Windows CI path retains all 30 filters from the final base and its launcher context.
The macOS path retains all 12 filters. These are filter counts, not individual test counts.
Both paths include the five task-evidence filters shown above; Windows also retains the added
issue-publication-journal and shared-copy-flow filters from the final base.

## Native confirmation process contract

The initial stage-2 review correctly found no native confirmation process check at
`70966a27f27429f9134e467c98b7b09a9894e39d`. It was present in PR #2686's stack at
`c63bec6e59b8c20e7247eb2ba597376d24805369`. The final base
`99238e8792b25312d2f1c6ad3e5a0954f1fe05c3` already includes both the script and its required
Linux/macOS workflow paths, so this check is part of the current prerelease contract.

The final integration preserves `scripts/check_native_confirmation_process.mjs` and forwards
identical feature, target, profile, and target-directory options to its production Gateway build
and exact ignored library test. Linux keeps default features; macOS uses all features and its
native target so the helper test can reuse the preceding native library test build.
Executable and library-test artifacts remain distinct; matching options does not remove that distinction.

Each Linux/macOS invocation runs exactly this one test with `--ignored --exact`:

`adapter::inbound::gateway::tests::human_confirmation::native_process::native_helper_process_confirms_replays_and_revokes_on_eof`

The script requires exactly one passing test. This explicit ignored-test path is separate from
the 30 Windows / 12 macOS normal contract filters and from the nextest inventory comparison.
Keep it required without enabling other ignored tests or replacing it with a normal filtered test.

## Verification and performance claims

The focused Node contract tests use fake Cargo/libtest results to verify artifact selection,
option forwarding, all Windows filters, failure propagation, and empty-filter rejection. They do
not run a model turn or compile the Rust package.

This change removes repeated Cargo preparation calls between native filters. It does not claim
that each previous call recompiled, nor claim a measured speedup. Compare Cargo timing artifacts,
compile counts, and elapsed/runner time under the same cache and build conditions before publishing
performance results.
