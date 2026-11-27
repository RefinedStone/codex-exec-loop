# Development and Delivery Guide

[한국어](../ko/reference/development.md)

This document consolidates the repository map, coding rules, tests, worktree coordination, and
GitHub delivery workflow. `AGENTS.md` remains the compact instruction entrypoint.

## Repository Map

- `src/core/`: headless app command/effect/completion/snapshot runtime
- `src/domain/`: pure conversation, session, planning, parallel, terminal, and review models
- `src/application/service/`: use-case orchestration and control-plane services
- `src/application/port/`: adapter-independent boundary contracts
- `src/application/port/inbound/`: adapter-facing use-case interfaces and request/response contracts
- `src/application/port/outbound/`: application-owned integration capability contracts
- `src/adapter/inbound/tui/`: alternate-screen fullscreen Ratatui/Crossterm shell
- `src/adapter/inbound/{cli,admin_api,telegram_bot}/`: other operator adapters
- `src/adapter/outbound/{app_server,db,filesystem,git,github,telegram}/`: concrete boundaries
- `src/composition/`: production dependency wiring
- `schema/`: checked-in app-server protocol snapshot and provenance, and the exported Akra kernel spec
  `akra-kernel-spec.json` (rewrite it with `bash scripts/refresh_akra_kernel_spec.sh` after a kernel
  protocol change; a test fails when it is stale)
- `templates/admin/`, `assets/admin/`: Admin UI templates and packaged assets
- `npm/`: launcher, platform packages, and Node tests
- `scripts/`: validation, packaging, GitHub identity, planning, and cleanup helpers
- `tests/`: cross-layer integration and architecture gates
- `docs/`: current references, executable validation contracts, explicit future plans, evidence,
  and compact pinned competitor research

## Coding Rules

- Write explicit Rust that a Spring Boot/Kotlin developer can scan quickly.
- Use small single-purpose functions and consistent `Service`, `Port`, `Adapter`, `Request`,
  `Response`, and `State` names.
- Prefer straightforward structs and methods over macro-heavy or speculative abstractions.
- Keep mapping in adapters and pure decisions in domain code.
- Make inbound adapters depend on narrow inbound ports; implement those ports in services/use cases.
- Return `Result` at fallible boundaries; avoid `panic!` outside tests.
- Add an outbound port only when a real integration boundary exists.
- Keep composition wiring near entrypoints; feature code must not import convenient concrete leaves.
- Import symbols explicitly in app runtime/controller/presentation/planning modules; avoid parent
  wildcard imports.

For TUI work, choose the owning layer before editing. Keep new context-facing modules near 600 LOC;
split mixed-responsibility files before they pass roughly 800 LOC.

## Commands and Tests

Local checks are a quick pre-flight; `CI Gate` is the full gate. Before each commit, run
`git diff --check`, and `cargo fmt --all` when Rust changed.

When Rust or Node source changed, run the deterministic change planner and executor. It uses the
shared base/head Cargo graph and ownership policy, includes staged, unstaged, untracked, renamed
and deleted inputs, and recollects their content identity immediately before execution. Unknown
inputs and root consumers retain the existing full gate. Skip it for docs, CI workflow, or
config-only changes: use the diff check, and let `CI Gate` prove the workflow.

```text
node scripts/agent-plan.mjs --explain
node scripts/agent-plan.mjs --run
```

`--json` emits a machine-readable plan without executing checks. Saved plans fix the source SHA,
local contents, base, graph, ownership and requested configuration; changing any of these requires
a fresh plan. Keep saved JSON under ignored `artifacts/` so the plan file does not become a new
validation input. See [local validation](ci-local-validation.md) for saved plans, PowerShell/POSIX
entrypoints, offline metadata, measurements and supported execution conditions.

Common commands are platform-neutral:

```text
cargo run
cargo build
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
```

The complete Rust suite and npm release archive tests require a POSIX environment. Run the broad
POSIX/CI gate with:

```bash
. "$HOME/.cargo/env"
bash scripts/check_native_pr.sh
```

On Windows, use the native PowerShell gate instead of invoking the POSIX script through Git Bash or
WSL:

```powershell
powershell -File scripts/check_native_pr.ps1 -Mode Full
```

If `cargo` or `rg` is not on `PATH` (no `~/.cargo/bin` proxies), reinstall `rustup` to restore them.
As a stopgap for one session, prepend
`$env:USERPROFILE\.rustup\toolchains\<rust-toolchain channel>-x86_64-pc-windows-msvc\bin` and a
folder that contains `rg.exe`.

The Windows gate runs native TUI policy checks, Windows-safe Node/admin checks, rustfmt, clippy, and
the code-deterministic Windows security/process/worktree contracts. `-Mode Rust` and `-Mode Admin`
provide proportional local subsets. `-Mode Portable` is the clean-runner CI mode and additionally
checks host executable trust/ACL behavior, which can legitimately reject a developer machine's
mutable Node or package-manager install. The full `cargo test --locked` suite remains Ubuntu/CI-owned
because many subprocess and filesystem tests intentionally assume a POSIX host.

The kernel simulation test (`src/composition/kernel_host/simulation.rs`) runs the real kernel host
with a fake clock, fake effect executor, and fake app-server, and checks kernel invariants for 24
seeds on every `cargo test`. Run more seeds with `AKRA_SIM_SEEDS=<count>`, and reproduce a reported
failure with `AKRA_SIM_SEED=<seed> cargo test --locked kernel_simulation`.

Tests run git through `crate::test_utils::git_command()` (or the one `git_command` helper of an
integration test file), never through a raw `Command::new("git")`. The helper drops every inherited
`GIT_*` variable. A test run started from a git hook or an agent shell can inherit `GIT_DIR`, and then
fixture `init`, `config user.*`, `commit`, and `reset` run in the real work repository; 123 commits
reached this repository with the fixture identity that way. An architecture test enforces the helper,
and `scripts/gh-akra.sh pr create` refuses commits, or a repository identity, on documentation
domains (`example.*`, `.example`, `.test`). The host-owned parallel worker identity
(`akra@localhost.invalid`) stays allowed.

Use unit tests beside modules and integration tests under `tests/`. Prioritize startup checks,
app-server parsing, stream reduction, session mapping, planning authority mutations, and parallel
recovery boundaries. Primitive-sensitive terminal changes also follow
[the validation methodology](../validation/terminal-ui-testing-methodology.md) and attach the
required real-terminal evidence.

### npm audit exceptions

The Node checks run `node scripts/npm-audit-gate.mjs <package directory>` for `assets/admin/game`
and `desktop`. It is `npm audit` at the `high` level with one difference: an advisory in
`scripts/npm-audit-exceptions.json` does not fail the check until its `expires` date. Use an
exception only when no patched version exists. Each entry names one advisory id (`GHSA-…`), the
package, the date, and the reason. After the date the check fails again; then update the package
and remove the entry, or look at the advisory again and set a new date.

## Cargo build storage and iteration speed

Normal `cargo build`, `cargo run`, and `cargo test` use an incremental development profile with
line-table debug information for Akra and no debug information for dependencies. The test profile
inherits dev. Akra panic backtraces retain source filenames and line numbers; local variables and
function arguments are unavailable in a debugger, and dependency frames may lack source locations.
Optimization, debug assertions, overflow checks, and release settings keep Cargo's defaults.

For variable inspection or stepping into dependencies, opt into full debug information:

```text
cargo build --locked --profile debugging
cargo test --locked --profile debugging --no-run
```

The `debugging` profile restores debug information for both Akra and its dependencies. It builds in
`target/debugging/` (under `CARGO_TARGET_DIR` when set), so returning to the normal commands reuses
`target/debug/`. The first use compiles a separate set of artifacts and uses additional space;
enable it only when that debugging session needs it.

Keep the active lane's target directory between edits, tests, and clippy runs. Keep its toolchain,
feature selection, and `RUSTFLAGS` stable where possible: changing build conditions creates different
artifacts. Honor an existing `CARGO_TARGET_DIR`; do not force parallel worktrees into one shared
target because Cargo's build lock can serialize them and source-path-specific artifacts still
differ. CI retains its existing `CARGO_INCREMENTAL=0` setting for ephemeral runners; local development
keeps incremental compilation. CI target cache keys include the manifest, lockfile, and toolchain so
a profile change can save and reuse a new cache rather than repeatedly restoring an incompatible one.

Changing the default profile causes a one-time rebuild of affected artifacts; it does not delete
old artifacts or impose a disk quota. Do not put `cargo clean`, age-based eviction, or a cache-size
purge in build/test/startup hooks. Preserve active caches and use the existing merged-worktree
cleanup below after integration. Retire old targets explicitly when their lane or toolchain is no
longer needed; a full clean makes the next build cold.

When evaluating further storage changes, compare `cargo test --locked --no-run` on the same source,
toolchain, and feature set in separate measurement targets. Record target size and elapsed time for
the cold build, unchanged repeat, and a single-source incremental rebuild. Keep those measurement
targets separate from active development caches.

See Cargo's [build performance guide](https://doc.rust-lang.org/cargo/guide/build-performance.html)
and [profile reference](https://doc.rust-lang.org/cargo/reference/profiles.html).

## Worktree Lane

Read-only diagnosis, review, and planning use the current checkout. Create a dedicated worktree only
before the first tracked mutation, normally from the latest `origin/prerelease`:

```bash
git fetch origin
git worktree add ../codex-exec-loop-worktrees/docs-native-platform-reference \
  -b codex/docs-native-platform-reference origin/prerelease
```

Use one user-visible outcome, one branch, and one PR per worktree. Multiple small commits may belong
to that outcome; a commit is not itself a reason to create another PR. Inspect `git worktree list`,
local branches, and open PRs before choosing a lane. Codex-authored branches use
`codex/<outcome-name>`. Prefer disjoint file ownership and identify exact collision files when
overlap is intentional.

Keep local `prerelease` checked out only in the integration checkout. Feature worktrees start from
`origin/prerelease`; do not branch from another in-flight feature unless the dependency is explicit.

Common hotspots include the TUI runtime/controller/presentation files, planning authoring/runtime
services, `docs/README.md`, and current product/architecture references. Re-check active lanes before
touching them.

## GitHub Identity

This repository uses the repo-local `RefinedStone` delivery identity. Before the first remote write:

```text
git config --get akra.githubLogin
```

On POSIX, verify API writes with `bash scripts/gh-akra.sh auth write-status`. On Windows, use the
native `gh api user --jq .login` check when Git Bash cannot bridge the Windows credential helper.
Both results must match `RefinedStone` before a remote write.

Do not stop only because global `GITHUB_TOKEN`, connector identity, or `gh auth status` differs.
Check the repo-local login and credential helper first. Never print credential values.

Repository-scoped Git credentials are for `git push`; GitHub API writes accept explicit token
variables or trusted `gh auth token` through the wrapper. The removed legacy credential scan is
unsupported and fails closed. If the intended login, repository, and token identity cannot be
verified, do not write remotely.

## Review and Integration

The normal completed slice is:

```text
commit -> push -> PR targeting prerelease -> CI Gate -> rebase merge -> cleanup
```

`prerelease` is protected by a repository ruleset: every update must arrive through a PR, the stable
`CI Gate` check must pass, and history remains linear. GitHub auto-merge and branch deletion are
enabled. Use rebase merge; do not push the integration checkout directly.

Explicitly dependent PRs may target their parent `codex/**` branch. The pull-request workflow
includes those target branches and compares the actual PR base/head SHAs with the same `CI Gate`.
Keep the push workflow limited to `main` and `prerelease`; a stacked PR uses its normal open or
head-update event. Verify a workflow change with the resulting PR run, not only local policy tests.

The workflow also emits a measured `Fast Gate` from the selected policy checks, while long Rust
tests and portable jobs continue in parallel and are aggregated by `CI Gate` and the push-only
`Post-Merge Gate`. `CI Gate` remains the required Ruleset context unless the user explicitly
approves a separate operational change with the evidence package from
[the rollout runbook](pr-validation-rollout.md). Code changes must not add bypass actors or weaken
protection.

Inspect review threads and address only correct, in-scope feedback. Rebase an existing PR only for
an actual conflict, a requested base refresh, or a dependency it needs; a harmless base advance does
not justify another full CI run. After changing the reviewed head, rerun proportional local checks
and push normally, using `--force-with-lease` only when that explicit rebase rewrote the branch.

## Cleanup

After the branch is integrated, run from the integration checkout:

```bash
bash scripts/cleanup_merged_worktrees.sh --apply \
  --branch codex/docs-native-platform-reference
```

The helper removes only a clean, merged, non-root worktree. It deletes the remote branch only when
the live remote tip equals the cleaned local tip or is merged into the base, and it leases the
delete on that tip. It prunes a worktree whose directory is gone before it deletes the branch, and
skips a locked one. Use `--force-dirty` only for one
explicit finished branch whose remaining churn is disposable. Never run this cleanup path for
`akra-agent/slot-*`; parallel runtime owns those worktrees, leases, and session details.
