# Repository Guidelines

## Scope
This file refines the root [`AGENT.md`](../AGENT.md) for `native/`. The current product direction is the Rust client, so optimize for the TUI and `codex app-server` flow first. Keep guidance here Rust-specific; do not add Python workflow notes.

## Project Structure & Module Organization
`native/` is a Rust crate rooted at `Cargo.toml`.

- `src/domain/`: pure models such as session summaries and startup diagnostics
- `src/application/service/`: use-case orchestration like `StartupService` and `ConversationService`
- `src/application/port/`: interfaces owned by the application layer
- `src/adapter/inbound/tui/`: Ratatui/Crossterm screens and event handling
- `src/adapter/outbound/`: `codex app-server` integration and filesystem adapters
- `schema/`: checked-in protocol snapshot used to pin app-server shapes
- `docs/`: current native design notes, roadmap, and backlog

Keep mapping logic in adapters, not domain models.

## Hexagonal Architecture
Dependency flow should point inward: `adapter -> application -> domain`. Inbound adapters translate user events into service calls. Application services orchestrate use cases and depend on ports defined in `src/application/port/`. Outbound adapters implement those ports and own process, stdio, JSON, and filesystem details. `domain/` stays free of TUI types, transport formats, and external I/O. When adding a new external capability, define the boundary as a port first, then implement it in an adapter.

## Build, Test, and Development Commands
- `cd native && . "$HOME/.cargo/env" && cargo run`: launch the TUI
- `cd native && . "$HOME/.cargo/env" && cargo build`: compile the crate
- `cd native && . "$HOME/.cargo/env" && cargo test`: run tests
- `cd native && . "$HOME/.cargo/env" && cargo fmt`: format source

Run these before opening a PR. Add `cargo clippy --all-targets --all-features -D warnings` when touching lint-sensitive code.

## Coding Style & Naming Conventions
Write Rust so a Spring Boot Kotlin developer can read it quickly. Use 4-space indentation, `snake_case` for functions/modules, and `PascalCase` for types. Prefer explicit names and straightforward structs over clever abstractions. Keep functions small and single-purpose. Use names such as `Service`, `Port`, `Adapter`, `Request`, `Response`, and `State` consistently.

## Testing Guidelines
Place unit tests next to the module with `#[cfg(test)] mod tests`. Add integration tests under `native/tests/` when a flow spans multiple layers. Prioritize startup checks, app-server response parsing, stream reduction, and session list mapping. New adapter or service logic should usually ship with tests.

## GitHub PR Auth
Use the repo-local RefinedStone identity for PR operations.

- use `bash ../scripts/gh-refinedstone.sh` for `pr create`, `pr view`, `pr edit`, and review replies
- do not use GitHub MCP tools for PR creation, PR comments, or review replies in this repo because they authenticate as `seungjoo-1ee`
- if the local RefinedStone token is unavailable, push code only and do not leave GitHub comments from the wrong account
- once a change reaches a reviewable milestone, the default is `commit -> push -> PR`; do not stop at a local commit unless the user explicitly says to hold
- after a PR merges or closes, start the next task from the latest base branch on a new feature branch instead of continuing on the old branch
- for final integration, do not use GitHub merge-commit flow; rebase locally, fast-forward the base branch with linear history, then close the PR after the base branch already contains the reviewed commits

## Parallel Worktree Rule
When multiple native slices move in parallel, treat worktree setup as part of the design work, not as an afterthought.

- create one git worktree per live branch, normally from the latest `origin/prerelease`
- keep one reviewable slice and one PR per worktree; do not mix unrelated backlog items in one branch
- inspect active local worktrees, unmerged branches, and open PRs before naming a new branch
- assume another unmerged worktree may already own a nearby file boundary and prefer a disjoint lane when two workers are active
- use names that expose location such as `feature/native-<lane>-<zone>-<slice>`, `fix/native-<lane>-<zone>-<slice>`, `docs/native-<lane>-<zone>-<slice>`, or `chore/native-<lane>-<zone>-<slice>`
- keep `prerelease` checked out in one integration checkout only; feature worktrees should rebase onto `origin/prerelease` without checking out local `prerelease`
- do not branch a new worktree from another in-flight feature branch unless the dependency is explicitly documented
- if overlap is intentional, document the expected conflict surface and resolve it consciously during rebase or merge
- before starting concurrent work, map the slice to [`docs/plan/04-worktree-branch-rules.md`](docs/plan/04-worktree-branch-rules.md) and the active split plan in [`docs/plan/11-parallel-worktree-plan.md`](docs/plan/11-parallel-worktree-plan.md)

## Working Rules
Use official Codex interfaces first: `codex app-server`, `codex exec`, and `codex exec resume`. Keep commits small and milestone-based. Do not introduce unnecessary traits; add a port only when it improves a real boundary. For TUI changes, include a screenshot or short terminal capture in the PR when practical.
