# Goal: Core Runtime Refactor

## References

Read first:

- `AGENTS.md`
- `new/docs/architecture/core-runtime-boundary-architecture.md`
- `new/docs/architecture/parallel-control-plane-architecture.md`
- `new/docs/plan/repository-wide-rebuild-roadmap.md`

## Objective

Introduce `src/core` as a headless `AppCoordinator` / `ApplicationRuntime`.

`core` is not a replacement for `domain`. It owns app-level orchestration state
and command/event flow currently mixed into TUI. TUI remains an inbound adapter
responsible for terminal input, overlay state, prompt buffer, cursor, selection,
rendering, and redraw cadence.

## Hard Rules

- `src/core` must not import `ratatui`, `crossterm`, `axum`, or Telegram update types.
- Outbound adapters must not call core.
- Application/domain must not depend on core.
- Domain must not receive TUI-only state.
- Core must not call DB/git/filesystem adapters directly; go through application services and ports.
- Background completion must re-enter core as an input event, not mutate TUI state directly.
- Do not move startup/session/conversation/parallel all in one PR.
- Preserve existing behavior unless a slice explicitly says otherwise.

## Operating Rules

- Base work on `origin/prerelease`.
- Use small branches and small reviewable slices.
- For each meaningful slice: commit -> push -> PR -> merge into `prerelease`.
- Use `bash scripts/gh-refinedstone.sh` for GitHub writes.
- Set repo-local git identity to `RefinedStone` / `chem.en.9273@gmail.com`.
- Do not revert user changes.
- If a slice becomes large, split it smaller.
- Update docs when code and architecture drift.

## Slice Order

### CORE-00: Core Skeleton

Add minimal core module:

- `src/core/mod.rs`
- `src/core/app/command.rs`
- `src/core/app/event.rs`
- `src/core/app/state.rs`
- `src/core/app/snapshot.rs`
- `src/core/app/controller.rs`
- Optional `src/core/runtime/mod.rs`

Add minimal types:

- `AppCommand`
- `CoreInput`
- `AppEvent`
- `AppState`
- `AppSnapshot`
- `CoreController`

Acceptance:

- No behavior change.
- No UI framework imports in core.
- Unit tests for basic state/snapshot construction.
- `cargo fmt`.
- Relevant `cargo test`.

### CORE-STARTUP-01: Startup State Model

Move/model startup transition in core:

- `AppCommand::RunStartupChecks`
- `CoreInput::Command(AppCommand::RunStartupChecks)`
- startup state: `Idle`, `Loading`, `Ready`, `Failed`
- startup snapshot
- core controller tests

Acceptance:

- Existing TUI startup behavior still works.
- Core tests cover Loading and Ready/Failed transition shape.

### CORE-STARTUP-02: Startup Effect Runner

Move startup background execution toward core:

- Core effect runner owns `StartupService::run_checks()`.
- Completion returns as `CoreInput::EffectCompleted`.
- Core emits `AppEvent::StartupChanged`.
- TUI consumes core event/snapshot for startup state where practical.

Acceptance:

- TUI no longer directly spawns startup check if feasible in this slice.
- Shell entrypoint/runtime startup tests pass.

### CORE-SESSION-01: Session Catalog Lifecycle

Move session catalog orchestration toward core:

- `AppCommand::LoadSessionCatalog`
- session state/snapshot
- core effect runner calls `SessionService::load_session_catalog`
- completion re-enters core input
- TUI keeps overlay open/close and selection/cursor only

Acceptance:

- TUI does not directly run session catalog service/effect where migrated.
- Session rendering behavior remains stable.

### CORE-CONVERSATION-00: Conversation Preparation

Prepare conversation lifecycle boundary:

- Identify app state vs TUI presentation state.
- Add read-only conversation projection to core snapshot if low risk.
- Keep prompt input/cursor in TUI.
- Do not move streaming submission unless previous slices are stable.

### CORE-PARALLEL-00: Parallel Boundary

Clarify or prepare boundary only:

- Keep existing parallel control-plane single-writer gate.
- Core may call it as an application runtime subcomponent.
- Do not redesign epoch/stale drop/wake coalescing.
- Do not add a second queue actor.

## Test Guidance

Run per slice:

- `cargo fmt`
- Docs-only: `git diff --check`
- Startup: `cargo test startup`, `cargo test shell_entrypoint`, `cargo test shell_runtime`
- Session: `cargo test session` and relevant TUI runtime tests
- Parallel changes: `cargo test parallel_mode`
- Broader changes: `cargo test` when practical

## Final Report

Report:

- completed slices
- PR URLs and merge status
- changed core files
- changed TUI/application files
- tests run
- remaining next slices
- risks or deferred items
