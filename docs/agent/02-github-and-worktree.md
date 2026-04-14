# GitHub And Worktree Guide

## GitHub Identity

All GitHub writes for this repo must authenticate as `RefinedStone`.

- Set repo-local commit identity before the first commit in a worktree: `git config user.name RefinedStone` and `git config user.email chem.en.9273@gmail.com`.
- Keep `origin` on `https://github.com/RefinedStone/codex-exec-loop.git`.
- Prefer the repo-local `.git/refinedstone-credentials`.
- If another `credential.helper` is inherited, override it in this repo's local `.git/config` only.
- Before the first push in an environment, verify `git credential fill` for `https://github.com/RefinedStone/codex-exec-loop.git` resolves `username=RefinedStone`.
- Use `bash scripts/gh-refinedstone.sh` for `pr create`, `pr view`, `pr edit`, and review replies.
- Do not use GitHub MCP tools for PR or review-thread writes in this repo because they authenticate as `seungjoo-1ee`.
- If a commit is created under another author or committer identity, rewrite the branch history to `RefinedStone <chem.en.9273@gmail.com>` before any push or further review activity.
- If the RefinedStone identity cannot be verified, do not push, open PRs, or leave GitHub comments from that environment.

## Delivery Default

- Once a change reaches a reviewable milestone, the default is `commit -> push -> PR`.
- Do not stop at a local commit unless the user explicitly says to hold.
- After a PR merges or closes, start the next task from the latest target base branch on a new feature branch.
- For final integration, do not use GitHub's merge-commit flow.
- Rebase locally, fast-forward the base branch with linear history, then close the PR after the base branch already contains the reviewed commits.

## Review Handling

- Inspect every new review comment and thread before changing code.
- Apply feedback only when it is logically correct and aligned with the chosen architecture and product direction.
- Fix correctness and low-cost maintainability issues that fit the current design.
- If a comment is wrong, stale, or pushes in the wrong direction, reply with a concise rationale instead of changing code.
- Reply on each review thread only when `bash scripts/gh-refinedstone.sh` can authenticate as `RefinedStone`.
- Commit and push the review response separately from the original milestone commit when practical.
- Rebase the feature branch onto the latest target base branch before merge.

## Parallel Worktrees

- Create one git worktree per live branch, normally from the latest `origin/prerelease`.
- Keep one reviewable slice and one PR per worktree.
- Inspect active local worktrees, unmerged branches, and open PRs before naming a new branch.
- Assume another unmerged worktree may already own a nearby file boundary and prefer a disjoint lane when two workers are active.
- Use names such as `feature/native-<lane>-<zone>-<slice>`, `fix/native-<lane>-<zone>-<slice>`, `docs/native-<lane>-<zone>-<slice>`, or `chore/native-<lane>-<zone>-<slice>`.
- Keep `prerelease` checked out in one integration checkout only. Feature worktrees should rebase onto `origin/prerelease` without checking out local `prerelease`.
- Do not branch a new worktree from another in-flight feature branch unless the dependency is explicitly documented.
- If overlap is intentional, document the expected conflict surface and resolve it consciously during rebase or merge.
- Before starting concurrent work, map the slice to `../plan/04-worktree-branch-rules.md` and the current snapshot in `../plan/11-parallel-worktree-plan.md`.
