# Worktree Branch Rules

This document defines how native phase-2 work should be split when more than one branch is active at the same time.

The goal is simple:
- keep each worktree reviewable on its own
- keep merge order predictable
- avoid two active branches fighting over the same hotspot files

Assume a second unmerged worker exists by default. The branch rule should help that worker infer where you are likely to edit before they open your diff.

## 1. Base Branch Roles

- `prerelease` is the default integration base for current native feature, fix, and docs work.
- `production` is for release stabilization and promotion only.
- `main` is the repository default branch and historical baseline, but it is not the default base for current native phase-2 work unless a task explicitly targets it.

## 2. Worktree Layout

- keep the main repository checkout as the integration reference checkout
- let that integration checkout be the only checkout that owns local `prerelease`
- create active branches in sibling worktrees, not nested directories inside the repository
- use a stable path pattern such as `../codex-exec-loop-worktrees/<branch-slug>`
- let one worktree own exactly one live branch
- do not reuse one worktree for multiple active branches

Git does not deadlock here. If a second worktree tries to check out `prerelease`, Git fails immediately because one branch cannot be checked out in two worktrees at once. Treat that as a design rule, not as a recoverable surprise.

Example:

```bash
git fetch origin
git worktree add ../codex-exec-loop-worktrees/feature-native-session-query-model \
  -b feature/native-session-query-model origin/prerelease
```

## 3. Branch Name Contract

- use `feature/native-<lane>-<zone>-<slice>` for feature and refactor slices
- use `fix/native-<lane>-<zone>-<slice>` for bug or regression fixes
- use `docs/native-<lane>-<zone>-<slice>` for documentation-only work
- use `chore/native-<lane>-<zone>-<slice>` for tooling, validation, or packaging support
- keep the `native-` prefix so the branch is clearly part of the Rust client track
- make `<lane>` match the workstream such as `shell`, `followup`, `session`, `runtime`, `github`, or `platform`
- make `<zone>` match the dominant ownership area such as `runtime`, `presentation`, `overlay`, `query`, `adapter`, `service`, or `validation`
- make `<slice>` describe one reviewable delivery unit
- do not use names such as `misc`, `tmp`, `wip`, `final`, `v2`, or date-only suffixes

Good examples:
- `feature/native-shell-runtime-split`
- `feature/native-session-query-model`
- `feature/native-github-poller-port`
- `docs/native-platform-validation-matrix`

The point of the name is not aesthetics. Another worker should be able to guess the likely ownership zone from the branch name alone.

## 4. Active Work Awareness Rule

Before creating a new branch, check which unmerged slices already exist.

Minimum preflight:

- `git worktree list`
- `git branch -vv`
- before the first push in an environment, verify RefinedStone for this repo only: keep `origin` on `https://github.com/RefinedStone/codex-exec-loop.git`, confirm `git credential fill` resolves `username=RefinedStone`, and scope any helper override to local `.git/config`
- inspect open PRs when GitHub access is available
- if GitHub access is unavailable, rely on local worktrees, tracked branches, and the active plan doc before opening a new branch
- identify which `lane` and `zone` are already occupied by another unmerged branch
- choose a disjoint `lane` or `zone` when two workers are active

If overlap is intentional:

- record the expected collision files in the task note or PR body
- keep the overlap small and deliberate
- resolve the rebase or merge conflict as part of the slice, not as surprise cleanup later

## 5. Slice Contract

Each live branch should satisfy all of the following:

- one branch owns one reviewable slice
- one slice has one primary goal, one main ownership area, and one verification story
- docs, tests, and small enabling refactors are allowed when they are required by that slice
- unrelated backlog items do not ride along just because the same files are already open

Reject the slice and split it again if it tries to combine:
- runtime continuity plus session browser work
- inline-shell migration plus approval/activity panels
- automation control work plus GitHub polling
- packaging validation plus unrelated TUI behavior changes

## 6. Parallel Start Rule

Before creating a new worktree, record or confirm the slice in the active plan doc.

Each slice should declare:
- branch name
- goal
- file ownership
- verification command set
- dependency on any earlier slice
- expected merge target

If two candidate slices want the same hotspot files, choose one of these options:
- land a small precursor extraction first
- keep them in the same lane and run them sequentially
- redefine one slice around a different file boundary

For two workers, prefer one hotspot-heavy lane and one disjoint support lane rather than two hotspot-heavy branches.

## 7. Dependency Rule

- by default, every worktree branches from the latest `origin/prerelease`
- feature worktrees should rebase with `git fetch origin && git rebase origin/prerelease` instead of switching to local `prerelease`
- do not stack one in-flight feature branch on top of another by default
- allow a dependent branch only when rebasing both slices onto `prerelease` would create more churn than the dependency itself
- when a dependent branch is unavoidable, write the parent branch and restack order into the detailed plan doc and the PR body

## 8. Hotspot Rule

These files are common collision points and should not receive unrelated semantic edits from two active worktrees at the same time:

- `src/adapter/inbound/tui/app.rs`
- `src/adapter/inbound/tui/app/app_runtime.rs`
- `src/adapter/inbound/tui/app/shell_rendering.rs`
- `src/adapter/inbound/tui/app/shell_presentation.rs`
- `src/adapter/inbound/tui/app/shell_controller.rs`
- `src/adapter/inbound/tui/app/followup_overlay_ui.rs`
- `src/adapter/inbound/tui/app/session_overlay_ui.rs`
- `docs/README.md`
- `docs/plan/10-inline-scrollback-shell.md`

If two lanes need one of these files, prefer a short precursor branch that extracts or stabilizes the shared seam first.

## 9. Review And Merge Rule

- once the slice is reviewable, push the branch and keep exactly one PR tied to it
- if a rebased branch already has a PR, push the new head with `--force-with-lease` so the PR matches the reviewed commit
- keep review-response commits separate from the original milestone commit when practical
- before final integration, rebase the worktree branch onto the latest `origin/prerelease`
- perform the final fast-forward of `prerelease` only from the integration checkout that already owns local `prerelease`
- do not use GitHub merge-commit flow; fast-forward local `prerelease` to the reviewed branch head, push `prerelease`, then close the PR only after `origin/prerelease` contains that head commit
- if PR creation is unavailable but the push identity is already verified as `RefinedStone`, push the branch and stop there; otherwise stop without pushing or commenting, then after merge or closure remove the worktree and delete the branch if it no longer has a job

Cleanup example:

```bash
git worktree remove ../codex-exec-loop-worktrees/feature-native-session-query-model
git branch -d feature/native-session-query-model
git push origin --delete feature/native-session-query-model
```

## 10. Worktree Checklist

1. Sync `origin/prerelease`.
2. Inspect active worktrees, unmerged branches, and open PRs.
3. Pick one slice from the detailed parallel plan.
4. Confirm that no other active slice owns the same hotspot files unless the overlap is intentional and documented.
5. Create a dedicated worktree and branch.
6. Keep the branch scoped to that slice only.
7. Rebase onto `origin/prerelease` from the feature worktree without checking out local `prerelease`.
8. Run the slice-specific verification commands before review.
9. Push the reviewable branch, open or update one PR, and keep the PR head aligned with any rebase.
10. Fast-forward merge from the integration checkout, push `prerelease`, then close the PR with linear history.
11. Remove the finished worktree before starting another slice in the same lane.
