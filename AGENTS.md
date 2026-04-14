# Repository Guidelines

## Scope

This file is the fast path for Codex in this repo. Read it first, then open only the referenced markdown that matches the task.

- Product: native-first Rust client on `codex app-server`
- Optimize for the TUI and `codex app-server` flow first
- Keep agent files compact; move detail into `docs/agent/`

## Quick Rules

- Layout: `src/domain`, `src/application/service`, `src/application/port`, `src/adapter/inbound/tui`, `src/adapter/outbound`, `schema`, and `docs`
- Architecture: `adapter -> application -> domain`; define ports before adding a real outbound boundary; keep mapping logic in adapters
- Style: explicit, Kotlin-readable Rust; small single-purpose functions; consistent `Service` / `Port` / `Adapter` / `Request` / `Response` / `State` naming
- Commands: source `"$HOME/.cargo/env"`, then run `cargo run`, `cargo build`, `cargo test`, or `cargo fmt`; add `cargo clippy --all-targets --all-features -D warnings` for lint-sensitive work
- Tests: unit tests beside modules, integration tests under `tests/`; focus on startup checks, app-server parsing, stream reduction, and session list mapping
- Working style: use official Codex interfaces first; keep commits small; add ports only for real boundaries; include a terminal capture in meaningful TUI PRs when practical
- GitHub writes: authenticate as `RefinedStone`, set repo-local `git config user.name RefinedStone` and `git config user.email chem.en.9273@gmail.com` before committing, keep `origin` on the RefinedStone repo, use `bash scripts/gh-refinedstone.sh` for PR and review-thread writes, and do not write to GitHub if identity is uncertain
- Delivery default: once a change is reviewable, finish with `commit -> push -> PR` unless the user says to hold locally
- Parallel work: one worktree and one reviewable slice per branch, usually from `origin/prerelease`; inspect active work before choosing a lane
- Do not expand this file into backlog or design notes; keep that in `docs/design` or `docs/plan`

## Open When Needed

- [`docs/agent/README.md`](./docs/agent/README.md)
- [`docs/agent/01-project-playbook.md`](./docs/agent/01-project-playbook.md)
- [`docs/agent/02-github-and-worktree.md`](./docs/agent/02-github-and-worktree.md)
- [`docs/plan/04-worktree-branch-rules.md`](./docs/plan/04-worktree-branch-rules.md)
- [`docs/plan/11-parallel-worktree-plan.md`](./docs/plan/11-parallel-worktree-plan.md)
