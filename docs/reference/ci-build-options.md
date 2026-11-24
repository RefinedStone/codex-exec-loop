# Native Rust validation build options

Issue: [#2690](https://github.com/RefinedStone/codex-exec-loop/issues/2690), stage 2.

## Build contracts

| Lane | Features | Target | Profile | Validation |
| --- | --- | --- | --- | --- |
| Linux complete Rust suite | default | runner host | Cargo test default | unit, integration, doctests |
| Linux Clippy | all | runner host | Cargo dev default | all targets |
| Windows portable | all | x86_64-pc-windows-msvc | Cargo defaults | production check + 23 existing library filters |
| macOS portable | all | aarch64-apple-darwin | Cargo defaults | production check + existing library filters |

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
  'application::service::evidence_vault_unlock::'
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
the Git Bash Windows CI path retains the same 23 filters and launcher context.

## Pending native confirmation stack

The native confirmation process check referenced by #2690 is present on
`c63bec6e59b8c20e7247eb2ba597376d24805369` (PR #2686's stack), but not on the stage-2 base
`70966a27f27429f9134e467c98b7b09a9894e39d`. It must be fixed in that stack rather than silently
bringing unrelated feature work into prerelease. Its production Gateway build and exact ignored
library test must receive identical feature, target, profile, and target-directory options.
The separate ignored test remains required; it must not be replaced by normal filtered tests.

## Verification and performance claims

The focused Node contract tests use fake Cargo/libtest results to verify artifact selection,
option forwarding, all Windows filters, failure propagation, and empty-filter rejection. They do
not run a model turn or compile the Rust package.

This change removes repeated Cargo preparation calls between native filters. It does not claim
that each previous call recompiled, nor claim a measured speedup. Compare Cargo timing artifacts,
compile counts, and elapsed/runner time under the same cache and build conditions before publishing
performance results.
