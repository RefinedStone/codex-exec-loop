# Development and Delivery Guide

[한국어](../ko/reference/development.md)

This document consolidates the repository map, coding rules, tests, worktree coordination, and
GitHub delivery workflow. `AGENTS.md` remains the compact instruction entrypoint.

## Repository Map

- `src/core/`: headless app command/effect/completion/snapshot runtime
- `src/domain/`: pure conversation, session, planning, parallel, terminal, and review models
- `src/application/service/`: use-case orchestration and control-plane services
- `src/application/port/`: adapter-independent boundary contracts
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
- `docs/`: current references, executable validation contracts, future plans, evidence, and
  competitor research

## Coding Rules

- Write explicit Rust that a Spring Boot/Kotlin developer can scan quickly.
- Use small single-purpose functions and consistent `Service`, `Port`, `Adapter`, `Request`,
  `Response`, and `State` names.
- Prefer straightforward structs and methods over macro-heavy or speculative abstractions.
- Keep mapping in adapters and pure decisions in domain code.
- Return `Result` at fallible boundaries; avoid `panic!` outside tests.
- Add an outbound port only when a real integration boundary exists.
- Keep composition wiring near entrypoints; feature code must not import convenient concrete leaves.
- Import symbols explicitly in app runtime/controller/presentation/planning modules; avoid parent
  wildcard imports.

For TUI work, choose the owning layer before editing. Keep new context-facing modules near 600 LOC;
split mixed-responsibility files before they pass roughly 800 LOC.

## Commands and Tests

```bash
. "$HOME/.cargo/env"
cargo run
cargo build
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets --all-features -- -D warnings
```

Use unit tests beside modules and integration tests under `tests/`. Prioritize startup checks,
app-server parsing, stream reduction, session mapping, planning authority mutations, and parallel
recovery boundaries.

For broad native/TUI work:

```bash
bash scripts/check_native_pr.sh
```

The gate runs TUI layering, Node surfaces, rustfmt, Rust tests, and clippy. Primitive-sensitive
terminal changes also follow [the validation methodology](../validation/terminal-ui-testing-methodology.md)
and attach the required real-terminal evidence.

## Worktree Lane

Every change uses a dedicated worktree, normally based on the latest `origin/prerelease`:

```bash
git fetch origin
git worktree add ../codex-exec-loop-worktrees/docs-native-platform-reference \
  -b docs/native-platform-reference origin/prerelease
```

Use one branch, one reviewable slice, and one PR per worktree. Inspect `git worktree list`, local
branches, and open PRs before choosing a lane. Prefer disjoint file ownership; document exact
collision files when overlap is intentional.

Branch patterns:

```text
feature/native-<lane>-<zone>-<slice>
fix/native-<lane>-<zone>-<slice>
docs/native-<lane>-<zone>-<slice>
chore/native-<lane>-<zone>-<slice>
```

Keep local `prerelease` checked out only in the integration checkout. Feature worktrees rebase onto
`origin/prerelease`; do not branch from another in-flight feature unless the dependency is explicit.

Common hotspots include the TUI runtime/controller/presentation files, planning authoring/runtime
services, `docs/README.md`, and current product/architecture references. Re-check active lanes before
touching them.

## GitHub Identity

This repository uses the repo-local `RefinedStone` delivery identity. Before the first remote write:

```bash
git config --get akra.githubLogin
bash scripts/gh-akra.sh auth write-status
```

Do not stop only because global `GITHUB_TOKEN`, connector identity, or `gh auth status` differs.
Check the repo-local login and credential helper first. Never print credential values.

Repository-scoped Git credentials are for `git push`; GitHub API writes accept explicit token
variables or trusted `gh auth token` through the wrapper. The removed legacy credential scan is
unsupported and fails closed. If the intended login, repository, and token identity cannot be
verified, do not write remotely.

## Review and Integration

The normal completed slice is:

```text
commit -> push -> PR targeting prerelease -> review -> rebase -> linear integration -> PR close
```

Before integration:

1. inspect every review thread and address only correct, in-scope feedback
2. `git fetch origin && git rebase origin/prerelease`
3. rerun the proportional verification gates
4. push the reviewed head (`--force-with-lease` only after a rebase of an existing PR)
5. fast-forward local `prerelease` from the integration checkout and push it
6. close the PR after the base contains the reviewed commits

Do not use a GitHub merge commit. Keep linear history and never reset unrelated user work.

## Cleanup

After the branch is integrated, run from the integration checkout:

```bash
bash scripts/cleanup_merged_worktrees.sh --apply \
  --branch docs/native-platform-reference
```

The helper removes only a clean, merged, non-root worktree. Use `--force-dirty` only for one
explicit finished branch whose remaining churn is disposable. Never run this cleanup path for
`akra-agent/slot-*`; parallel runtime owns those worktrees, leases, and session details.
