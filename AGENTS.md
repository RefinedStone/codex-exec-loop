# Repository Agent Guide

## Fast Path

- Read-only diagnosis, review, and planning stay in the current checkout; they do not require a
  worktree or delivery.
- Before the first tracked mutation, inspect active work, fetch `origin/prerelease`, and create one
  dedicated worktree from it.
- One user-visible outcome uses one worktree, branch, and PR. Keep commits small inside that PR;
  do not create a PR for every commit.
- Local gate when Rust or Node source changed: in the worktree, run `node scripts/agent-plan.mjs`
  and run exactly the checks it prints; the gate is done when they pass. For docs, CI workflow, or
  config-only changes, the gate is `git diff --check`, and `CI Gate` decides the rest. `CI Gate` is
  the full gate, so the broad scripts below run only when the planner prints them or the user asks.
- Before each commit, run `git diff --check`, and `cargo fmt --all` when Rust changed (again after
  any scripted edit).
- `desktop/` is outside the planner and CI: when it changed, also run
  `cd desktop && npm run check && npx vitest run`, and `npm run e2e` for UI changes.
- Unless the user asks to hold locally, finish reviewable changes with
  `commit -> push -> PR(prerelease) -> CI Gate -> rebase merge -> worktree cleanup`.

## Product and Architecture

- Product: Akra, a native-first Rust operator on `codex app-server`; operator command: `akra`.
- Direction: one kernel per repository owns conversation state and decisions (plan 18). TUI,
  Desktop (`desktop/`, Electron), CLI, and the Gateway daemon are its clients; behavior goes into the
  kernel and application services, and clients stay thin.
- Kernel and core changes move the existing kernel one property at a time (plan 19) and keep the
  kernel simulation test (`src/composition/kernel_host/simulation.rs`) green.
- Dependency direction is `adapter/inbound -> core or application -> domain`. Define ports only for
  real outbound boundaries and keep mapping in adapters.
- Primary layout: `src/{core,domain,application,adapter,composition}`, `desktop`, `schema`,
  `templates`, `assets`, `scripts`, `tests`, and `docs`.
- Write explicit, Kotlin-readable Rust with small functions and consistent
  `Service` / `Port` / `Adapter` / `Request` / `Response` / `State` names.

## Validation and Delivery

- Unit tests live beside modules; integration and architecture gates live under `tests/`.
- Broad POSIX/CI validation: `bash scripts/check_native_pr.sh`.
- Broad Windows validation: `powershell -File scripts/check_native_pr.ps1 -Mode Full`.
- Validate app-server behavior without model turns (build, `initialize`, `thread/list`,
  `thread/start`, the fake Gateway). Send `turn/start` or `turn/steer` only after the user allows it.
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
- Kernel direction and slice order: [`docs/plan/18-akra-kernel-architecture.md`](./docs/plan/18-akra-kernel-architecture.md),
  [`docs/plan/19-akra-kernel-quality-goals.md`](./docs/plan/19-akra-kernel-quality-goals.md)
- Desktop and observe-and-offer: [`docs/plan/17-akra-desktop/`](./docs/plan/17-akra-desktop/),
  [`docs/plan/20-akra-observe-and-offer.md`](./docs/plan/20-akra-observe-and-offer.md)
- [`docs/design/07-tui-layered-architecture-and-aesthetic-contract.md`](./docs/design/07-tui-layered-architecture-and-aesthetic-contract.md)
- [`docs/validation/terminal-ui-testing-methodology.md`](./docs/validation/terminal-ui-testing-methodology.md)
