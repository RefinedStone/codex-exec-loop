# Agent Entrypoint

This is the single canonical fast path for Codex in this repo. `AGENT.md` is a
compatibility pointer only.

## Work Rules
- 모든 의미있는 작업단위로 commit -> push -> rebase merge(prerelease) 과정을 거치세요

## Review guidelines

- Using Korean language when you're reviewing except code-block

# Repository Guidelines

## Scope

Read this first, then open only the referenced markdown that matches the task.

- Product: native-first Rust client on `codex app-server`
- Operator command: `akra`
- Optimize for the TUI and `codex app-server` flow first; keep CLI, admin API, Telegram, and automation surfaces on the same application services
- Keep agent files compact; move detail into `docs/agent/`

## Quick Rules

- Layout: `src/domain`, `src/application/service`, `src/application/port`, `src/adapter/inbound/{tui,cli,admin_api,telegram_bot}`, `src/adapter/outbound/{app_server,db,filesystem,git,github,telegram}`, `schema`, `templates`, `assets`, `scripts`, and `docs`
- Architecture: `adapter -> application -> domain`; define ports before adding a real outbound boundary; keep mapping logic in adapters
- Current implementation: Ratatui/Crossterm inline shell, app-server runtime, SQLite planning authority store, filesystem planning workspace, admin web UI/API, Telegram bot control plane, Git/GitHub parallel-mode delivery, npm/native release packaging
- Style: explicit, Kotlin-readable Rust; small single-purpose functions; consistent `Service` / `Port` / `Adapter` / `Request` / `Response` / `State` naming
- Commands: source `"$HOME/.cargo/env"`, then run `cargo run`, `cargo build`, `cargo test`, or `cargo fmt`; add `cargo clippy --all-targets --all-features -D warnings` for lint-sensitive work; use `bash scripts/check_native_pr.sh` before broad native/TUI PRs
- Tests: unit tests beside modules, integration tests under `tests/`; focus on startup checks, app-server parsing, stream reduction, and session list mapping
- Working style: use official Codex interfaces first; keep commits small; add ports only for real boundaries; include a terminal capture in meaningful TUI PRs when practical
- GitHub writes: authenticate as `RefinedStone`, set repo-local `git config user.name RefinedStone` and `git config user.email chem.en.9273@gmail.com` before committing, keep `origin` on the RefinedStone repo, use
  `bash scripts/gh-refinedstone.sh` for PR and review-thread writes, and do not write to GitHub if identity is uncertain
- Delivery default: once a change is reviewable, finish with `commit -> push -> PR` unless the user says to hold locally
- Parallel work: one worktree and one reviewable slice per branch, usually from `origin/prerelease`; inspect active work before choosing a lane
- Worktree cleanup: after a branch is merged into `prerelease`, remove the finished worktree from the integration checkout. Prefer `bash scripts/cleanup_merged_worktrees.sh --apply --branch <finished-branch>` for the lane you just integrated, but never for `akra-agent/slot-*` parallel-mode slot branches. If the lane is fully disposable but the repo still reports dirty CRLF or local churn noise, use `--force-dirty` explicitly for that finished branch only.
- Do not expand this file into backlog or design notes; keep that in `docs/design` or `docs/plan`

## Open When Needed

- [`docs/agent/README.md`](./docs/agent/README.md)
- [`docs/agent/01-project-playbook.md`](./docs/agent/01-project-playbook.md)
- [`docs/agent/02-github-and-worktree.md`](./docs/agent/02-github-and-worktree.md)
- [`docs/plan/04-worktree-branch-rules.md`](./docs/plan/04-worktree-branch-rules.md)
- [`docs/plan/11-parallel-worktree-plan.md`](./docs/plan/11-parallel-worktree-plan.md)
