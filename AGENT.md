# AGENT.md

## Repository Baseline

`codex-exec-loop` is a native-first Rust client built on `codex app-server`. Optimize for the TUI and `codex app-server` flow first.

## Core Rules

- Follow hexagonal flow: `adapter -> application -> domain`.
- Keep mapping logic in adapters, not domain models.
- Write explicit Rust that a Spring Boot Kotlin developer can read quickly.
- Use official Codex interfaces first.
- Verify native changes with `cargo fmt`, `cargo build`, and `cargo test`.
- GitHub writes must authenticate as `RefinedStone`; use the repo workflow reference before pushing or opening PRs.
- Keep commits small and milestone-based. The default reviewable finish line is `commit -> push -> PR` unless the user says to hold locally.
- After a task branch is integrated into `prerelease`, remove its finished feature worktree from the integration checkout instead of leaving it behind. If a branch is intentionally abandoned without merge, remove that worktree manually as part of the closeout.
- Keep this file lightweight. Move detail into `docs/agent/` or the existing `docs/design` and `docs/plan` trees.

## References

- [`AGENTS.md`](./AGENTS.md): fast repo-specific execution summary
- [`docs/agent/README.md`](./docs/agent/README.md): reference map
