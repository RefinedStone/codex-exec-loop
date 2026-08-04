# Repository Agent Guide

## Fast Path

- Read-only diagnosis, review, and planning stay in the current checkout; they do not require a
  worktree or delivery.
- Before the first tracked mutation, inspect active work, fetch `origin/prerelease`, and create one
  dedicated worktree from it.
- One user-visible outcome uses one worktree, branch, and PR. Keep commits small inside that PR;
  do not create a PR for every commit.
- Before delivery, run `node scripts/agent-plan.mjs` and the proportional checks it prints.
- Unless the user asks to hold locally, finish reviewable changes with
  `commit -> push -> PR(prerelease) -> CI Gate -> rebase merge -> worktree cleanup`.

## Product and Architecture

- Product: native-first Rust client on `codex app-server`; operator command: `akra`.
- Optimize the TUI/app-server flow first. CLI, admin API, Telegram, and automation reuse the same
  application services.
- Dependency direction is `adapter/inbound -> core or application -> domain`. Define ports only for
  real outbound boundaries and keep mapping in adapters.
- Primary layout: `src/{core,domain,application,adapter,composition}`, `schema`, `templates`,
  `assets`, `scripts`, `tests`, and `docs`.
- Write explicit, Kotlin-readable Rust with small functions and consistent
  `Service` / `Port` / `Adapter` / `Request` / `Response` / `State` names.

## Validation and Delivery

- Unit tests live beside modules; integration and architecture gates live under `tests/`.
- Broad POSIX/CI validation: `bash scripts/check_native_pr.sh`.
- Broad Windows validation: `powershell -File scripts/check_native_pr.ps1 -Mode Full`.
- For meaningful TUI changes, follow the terminal validation contract and include a capture when
  practical.
- Use official Codex interfaces first. Review feedback is written in Korean except code blocks.
- GitHub writes use the repo-local `RefinedStone` identity described in the development guide.
  Never print credentials or stop only because a global token/account differs.
- After merge, run the explicit cleanup helper from the integration checkout. Never use it for
  `akra-agent/slot-*` runtime worktrees.
- Keep this file compact. Shipped detail belongs in `docs/reference/`; future work must be marked as
  proposed.

## Open Only When Needed

- [`docs/agent/README.md`](./docs/agent/README.md)
- [`docs/reference/current-product.md`](./docs/reference/current-product.md)
- [`docs/reference/architecture.md`](./docs/reference/architecture.md)
- [`docs/reference/development.md`](./docs/reference/development.md)
- [`docs/design/07-tui-layered-architecture-and-aesthetic-contract.md`](./docs/design/07-tui-layered-architecture-and-aesthetic-contract.md)
- [`docs/validation/terminal-ui-testing-methodology.md`](./docs/validation/terminal-ui-testing-methodology.md)
