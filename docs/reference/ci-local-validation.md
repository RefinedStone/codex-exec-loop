# Local validation from the shared plan

The local entrypoint uses the [shared impact planner](ci-impact-plan.md) and its base/head Cargo
dependency graphs. Planning and execution are separate operations. A selected result covers the
displayed packages, behavioral contracts and Node surfaces; it is not a replacement for CI Gate.

## Plan, inspect and run

Run from a repository worktree with Node, Git, Cargo, rustc and tar available:

```text
node scripts/agent-plan.mjs --explain
node scripts/agent-plan.mjs --json
node scripts/agent-plan.mjs --run
```

No action flag means `--explain`. Explain prints reasons, excluded packages, full fallback reasons
and structured argv. JSON writes only the plan to stdout. Neither action compiles or runs tests.
`--run` recollects inputs before the first command, verifies the plan hash, executes each command
without shell evaluation, stops on the first failure, and checks input identity again afterward.

The base defaults to the merge base of `origin/prerelease` and HEAD. Use `--base <ref>` to choose
another base. `--features default|all|both`, `--profile <name>` and `--target <native-triple>` fix
the configuration in the plan. Both feature modes are the default. Local execution accepts one OS
and at most one target triple; it rejects cross-OS or cross-architecture execution.

Metadata uses `cargo metadata --locked --all-features --offline` by default. Cargo must already
have the required dependency index/cache and toolchain. `--online` permits Cargo's locked metadata
resolution to download missing cache entries. Neither mode runs a build or changes the lockfile.
An unusable head inventory is an error, since no trustworthy command list can be constructed.
An unavailable base ref, old registry, or old dependency cache produces a full fallback.

Keep saved JSON inside an ignored directory:

```bash
mkdir -p artifacts/local-validation
node scripts/agent-plan.mjs --json > artifacts/local-validation/plan.json
node scripts/agent-plan.mjs --plan artifacts/local-validation/plan.json --explain
node scripts/agent-plan.mjs --plan artifacts/local-validation/plan.json --run
```

Windows PowerShell users should write UTF-8 explicitly; PowerShell 5's default redirection is
UTF-16. A UTF-8 BOM is accepted:

```powershell
New-Item -ItemType Directory -Force artifacts/local-validation | Out-Null
node scripts/agent-plan.mjs --json | Set-Content -Encoding UTF8 artifacts/local-validation/plan.json
node scripts/agent-plan.mjs --plan artifacts/local-validation/plan.json --run
```

Do not combine a saved plan with new base, feature, profile or target arguments. Generate a new
plan instead. A saved plan in an ordinary untracked path changes its own source inventory and will
be rejected as stale. Staging, unstaging, rebasing, editing ownership or changing build settings
also invalidates the prior plan.

## Existing shell entrypoints

```bash
bash scripts/check_native_pr.sh --changed --base origin/prerelease
bash scripts/check_native_pr.sh --full
```

```powershell
powershell -File scripts/check_native_pr.ps1 -Mode Changed -Base origin/prerelease
powershell -File scripts/check_native_pr.ps1 -Mode Changed -PlanFile artifacts/local-validation/plan.json
powershell -File scripts/check_native_pr.ps1 -Mode Full
```

PowerShell's optional `-FeatureMode`, `-ValidationProfile` and `-TargetTriple` parameters apply
only to `-Mode Changed`. Existing Full, Rust, Admin and Portable modes keep their meaning.
`check_windows_portable.sh --changed` forwards to the same local planner with the explicit
`x86_64-pc-windows-msvc` target. Use the Node entrypoint for saved-plan execution.
Calling either shell script without arguments still runs its existing broad gate.

`node scripts/agent-plan.mjs --run --full` explicitly delegates to the existing full native gate.
A fallback also delegates to that gate, never to an empty success. The full gate uses its standard
test profile and host configuration; unsupported custom full-gate settings are an error.

## Coverage and preserved build state

An independent selected Rust member runs its complete registered unit, binary and integration
targets plus doctests through `run_native_rust_contracts.mjs --include-members --package <name>`.
Registered contracts run in their separate processes, followed by the exact remaining tests.
The full package suite is not run a second time after its contracts. The planner requests both
default and all features unless explicitly configured otherwise. Selected packages also run
Clippy; the Rust path includes formatting and crate boundary checks.

Unknown paths, manifest/lock changes, incomplete graphs, or affected root consumers keep the full
fallback. The currently extracted issue-core package still has a root consumer: this change does
not authorize omitting that consumer. Unsupported selective target kinds, feature-gated default
coverage, contract isolation or contract feature combinations are explicit errors with the option
to request the full gate.

Node surfaces use the existing Node/admin entrypoint. A documentation-only plan runs the diff
check. The repository's lightweight docs/config-only commit rule remains in AGENTS.md.

The executor inherits `CARGO_TARGET_DIR` and `CARGO_INCREMENTAL`. It never cleans a target or
shares another worktree's target automatically. Commands reuse `ci-performance.mjs`; records go
to `artifacts/ci-performance/steps.jsonl` with the plan prefix, step label and configuration.
Contract progress retains the existing native runner format. Compare repeated measurements only
under matching toolchain, target, features, profile and cache conditions; see the
[pre-extraction local baseline](../validation/ci-local-build-baseline-2026-11-27.md).

The existing streaming process supervisor owns the measurement wrapper's process group/tree.
Interrupts, termination, nonzero child status and failed descendant cleanup cannot be reported as
a successful local validation.

## Input collection and CI adapter

`collectPlanInput({cwd, baseRef, baseSha, headSha, configuration, offline = true,
requireCleanCheckout = false, environment})` returns a request for `createValidationPlan`.
An explicit base SHA is immutable; a requested head SHA must match the checkout. The caller owns
JSON persistence and execution. The collector archives the base into its own temporary directory,
reads full metadata and the registry there, checks its lockfile, then removes only that temporary
directory. It does not change the working checkout, index or target.

The local content identity includes HEAD, index blobs/stages, tracked and untracked file bytes,
both rename sides, deletions, ignored registered inputs, Cargo configuration files, toolchain
versions and compiler/tool discovery environment. Values such as environment secrets remain only
inside the digest. File hashing checks ordinary-file identity before and after reading; symlinks
and submodules require a future explicit policy and are rejected.

Ignored files under registered directory prefixes are included, as are exact external/generated
inputs. The registry explicitly identifies `assets/admin/game/node_modules/` as an npm-ci install
output and binds that exception to its tracked package manifest and lockfile. This directory sits
inside the broad external `assets/` input; producing it during the full gate must not invalidate
unchanged source. Only ignored files in that fixed reviewed install directory are excluded.
Tracked files remain inputs, and any explicit source, generated or contract input overlapping the
output makes the policy fail. Arbitrary output prefixes or untracked manifest/lock inputs fail too.

Other registered ignored inputs remain fingerprinted. A broad input prefix can therefore make
hashing expensive when undeclared outputs sit below it. Cache directories outside these input
prefixes are not recursively fingerprinted. If ancestor Cargo configuration cannot be reproduced
for the temporary base, the base remains incomplete and validation falls back to full.

CI scope can use `offline: false` on a fresh runner and `requireCleanCheckout: true`.
The synchronous `assertCleanPlanCheckout({cwd, environment})` helper returns
`{sourceSha, clean: true}` or throws. It requires clean staged/unstaged/untracked status and also
rejects existing ignored inputs registered by exact path or directory prefix, except the reviewed
install output above. This prevents source bytes from bypassing a clean Git status. CI workers verify the immutable source and plan
conditions separately; local environment/path hashes are not claimed to be portable across OSes.
