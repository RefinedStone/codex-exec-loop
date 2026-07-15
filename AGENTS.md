## Work Rules
- 모든 변경이 있는 작업은 worktree를 생성하여 진행하세요
- 모든 의미있는 작업단위로 commit -> push -> PR(prerelease) -> rebase merge(prerelease) 과정을 거치세요
- 작업 이후에는 worktree를 삭제하세요

## Review guidelines

- Using Korean language when you're reviewing except code-block

# Repository Guidelines

## Scope

Read this first, then open only the referenced markdown that matches the task.

- Product: native-first Rust client on `codex app-server`
- Operator command: `akra`
- Optimize for the TUI and `codex app-server` flow first; keep CLI, admin API, Telegram, and automation surfaces on the same application services
- Keep agent files compact; move current implementation detail into `docs/reference/`

## Quick Rules

- Layout: `src/core`, `src/domain`, `src/application/service`, `src/application/port`, `src/adapter/inbound/{tui,cli,admin_api,telegram_bot}`, `src/adapter/outbound/{app_server,db,filesystem,git,github,telegram}`, `schema`, `templates`, `assets`, `scripts`, and `docs`
- Architecture: `adapter/inbound -> core or application -> domain`; define ports before adding a real outbound boundary; keep mapping logic in adapters
- Current implementation: Ratatui/Crossterm inline shell, headless core runtime, app-server runtime, SQLite planning authority store, filesystem planning workspace, admin web UI/API, Telegram bot control plane, Git/GitHub parallel-mode control-plane delivery, npm/native release packaging
- Style: explicit, Kotlin-readable Rust; small single-purpose functions; consistent `Service` / `Port` / `Adapter` / `Request` / `Response` / `State` naming
- Commands: source `"$HOME/.cargo/env"`, then run `cargo run`, `cargo build`, `cargo test`, or `cargo fmt`; add `cargo clippy --all-targets --all-features -D warnings` for lint-sensitive work; use `bash scripts/check_native_pr.sh` before broad native/TUI PRs
- Tests: unit tests beside modules, integration tests under `tests/`; focus on startup checks, app-server parsing, stream reduction, and session list mapping
- Working style: use official Codex interfaces first; keep commits small; add ports only for real boundaries; include a terminal capture in meaningful TUI PRs when practical
- GitHub writes: 이 저장소는 repo-local Git credential의 `RefinedStone` 계정으로 push 및 PR 생성이 가능하다. 전역 `GITHUB_TOKEN`, MCP 계정 또는 `gh auth status` 불일치만으로 중단하지 말고, 먼저 `akra.githubLogin=RefinedStone`과 repo-local credential을 확인한 뒤 `bash scripts/gh-akra.sh auth write-status`로 검증하라. repo-local 검증까지 실패한 경우에만 GitHub 쓰기가 불가능하다고 보고하며 자격증명 값은 절대 출력하지 않는다.
- Delivery default: once a change is reviewable, finish with `commit -> push -> PR` unless the user says to hold locally
- Parallel work: one worktree and one reviewable slice per branch, usually from `origin/prerelease`; inspect active work before choosing a lane
- Worktree cleanup: after a branch is merged into `prerelease`, remove the finished worktree from the integration checkout. Prefer `bash scripts/cleanup_merged_worktrees.sh --apply --branch <finished-branch>` for the lane you just integrated, but never for `akra-agent/slot-*` parallel-mode slot branches. If the lane is fully disposable but the repo still reports dirty CRLF or local churn noise, use `--force-dirty` explicitly for that finished branch only.
- Do not expand this file into backlog or design notes; keep shipped truth in `docs/reference/`, executable TUI/validation contracts in their guarded docs, and future work explicitly marked as proposed

## Open When Needed

- [`docs/agent/README.md`](./docs/agent/README.md)
- [`docs/reference/current-product.md`](./docs/reference/current-product.md)
- [`docs/reference/architecture.md`](./docs/reference/architecture.md)
- [`docs/reference/development.md`](./docs/reference/development.md)
- [`docs/design/07-tui-layered-architecture-and-aesthetic-contract.md`](./docs/design/07-tui-layered-architecture-and-aesthetic-contract.md)
- [`docs/validation/terminal-ui-testing-methodology.md`](./docs/validation/terminal-ui-testing-methodology.md)
