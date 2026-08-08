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
- `schema/`: checked-in app-server protocol snapshot and provenance
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

Start with the deterministic change planner. It classifies the current diff using the same policy as
GitHub Actions and prints the smallest safe local gate:

```text
node scripts/agent-plan.mjs
```

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

The Windows gate runs native TUI policy checks, Windows-safe Node/admin checks, rustfmt, clippy, and
the code-deterministic Windows security/process/worktree contracts. `-Mode Rust` and `-Mode Admin`
provide proportional local subsets. `-Mode Portable` is the clean-runner CI mode and additionally
checks host executable trust/ACL behavior, which can legitimately reject a developer machine's
mutable Node or package-manager install. The full `cargo test --locked` suite remains Ubuntu/CI-owned
because many subprocess and filesystem tests intentionally assume a POSIX host.

Use unit tests beside modules and integration tests under `tests/`. Prioritize startup checks,
app-server parsing, stream reduction, session mapping, planning authority mutations, and parallel
recovery boundaries. Primitive-sensitive terminal changes also follow
[the validation methodology](../validation/terminal-ui-testing-methodology.md) and attach the
required real-terminal evidence.

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

The helper removes only a clean, merged, non-root worktree. Use `--force-dirty` only for one
explicit finished branch whose remaining churn is disposable. Never run this cleanup path for
`akra-agent/slot-*`; parallel runtime owns those worktrees, leases, and session details.
