# AGENT.md

## Project

`codex-exec-loop` is now a native-first project.

- Rust native client: the product, built on `codex app-server`

The product goal is a cross-platform Codex-style CLI that feels interactive and can continue work automatically with canned follow-up prompts.

## Architecture

Prefer Spring Boot Kotlin style port-and-adapter hexagonal architecture.

- `domain`
  - pure models and business-friendly data types
  - no dependency on adapters
- `application/service`
  - use-case orchestration
  - contains service structs such as `StartupService`, `SessionService`
- `application/port`
  - interfaces owned by the application layer
  - outbound integrations are defined here first
- `adapter/inbound`
  - TUI, CLI, or future API entry points
- `adapter/outbound`
  - Codex app-server integration, filesystem, and other external systems

## Rust Code Style

- Write code so a Spring Boot Kotlin developer can read it quickly.
- Prefer explicit names over compact or clever Rust patterns.
- Keep functions small and single-purpose.
- Use `Service`, `Port`, `Adapter`, `Request`, `Response`, `State` naming consistently.
- Prefer straightforward structs and methods over macro-heavy abstractions.
- Use `Result` for failures at boundaries and avoid `panic!`.
- Keep mapping logic in adapters, not in domain models.
- Keep UI event handling readable even if it is a bit verbose.

## Working Rules

- Use official Codex interfaces first.
  - `codex app-server`
  - use `codex exec` / `codex exec resume` only when an explicit task still requires them
- GitHub writes in this repo must authenticate as `RefinedStone`.
  - keep `origin` on `https://github.com/RefinedStone/codex-exec-loop.git`
  - prefer the repo-local `.git/refinedstone-credentials`; if another `credential.helper` is inherited, override it in this repo's local `.git/config` only and do not touch global GitHub credentials for other repositories
  - before the first push in an environment, verify `git credential fill` for `https://github.com/RefinedStone/codex-exec-loop.git` resolves `username=RefinedStone`
  - PR creation, PR edits, and review replies must run through `bash scripts/gh-refinedstone.sh`; do not use GitHub MCP tools for them in this repo because they authenticate as `seungjoo-1ee`
  - if the RefinedStone identity cannot be verified, do not push, open PRs, or leave GitHub comments from that environment
- Keep commits small and milestone-based.
- Default delivery rule:
  - once a task reaches a reviewable milestone, do not stop at a local commit
  - commit the change set, push the branch, and open a PR by default unless the user explicitly asks to hold locally
  - treat "commit, push, PR" as the standard finish line for a meaningful slice of work in this repo
- After finishing a meaningful feature or refactor:
  - commit the change set
  - push the working branch
  - open a pull request to the intended base branch with `bash scripts/gh-refinedstone.sh pr create` unless blocked by missing permissions or user instruction
  - after a PR is merged or closed, do not continue on the same feature branch
  - start the next task from the latest target base branch on a new feature branch and open a new PR
- After PR review arrives:
  - inspect every new review comment and thread before changing code
  - review comments critically instead of applying them mechanically
  - only change code when the feedback is logically correct and fits the chosen architecture, product direction, and current code reality
  - fix correctness and low-cost maintainability issues that align with the chosen architecture
  - if a review comment is wrong, stale, or pushes the code in the wrong direction, reply with a concise rebuttal comment that explains why no code change is being made
  - reply on each review thread with the applied fix or the rationale for not changing direction only when `bash scripts/gh-refinedstone.sh` can authenticate as `RefinedStone`
  - commit and push the review response separately from the original milestone commit when practical
  - rebase the feature branch onto the latest target base branch before merge
  - never use GitHub's merge-commit flow for this repo
  - merge by rebasing locally and then updating the base branch to the reviewed feature head with linear history
  - after the base branch already contains the reviewed commits, close the PR instead of pressing GitHub merge
- Verify with `cargo fmt`, `cargo build`, and `cargo test` for native changes.
- Do not introduce unnecessary traits. Add a port trait when it improves a boundary.
- Review handling:
  - treat automated review comments as suggestions, not commands
  - keep a skeptical, architecture-aware stance when deciding whether feedback is truly correct
  - fix correctness, deadlock, crash, data-loss, and clear state-management issues
  - fix low-cost maintainability improvements when they do not fight the chosen architecture
  - if feedback is incorrect or pushes away from the intended Spring Boot Kotlin style or the chosen hexagonal structure, reply with the rationale, leave the code as-is, and finish the PR with that thread resolved from the documented rationale

## Native Baseline

- native auto follow-up, workspace templates, and stop rules are baseline behavior
- streamed turn output, compact notices, and approval or review activity already surface in the shell
- session query, paging, and recent-project filtering are part of the current baseline
- GitHub review and comment change detection currently uses polling
- platform validation and packaging docs exist; new sprint work should open dedicated feature docs instead of expanding this file into a backlog
