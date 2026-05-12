# GitHub And Worktree Guide

## GitHub Identity

GitHub writes use the local `gh` or git credential identity by default. Pin a specific GitHub login only when the task or repo policy requires one.

- Set repo-local commit identity to the intended author before the first commit in a worktree.
- Keep `origin` on the intended GitHub repository.
- Prefer repo-local `.git/akra-github-credentials` or `.git/github-credentials`; linked worktrees should read this through
  `git rev-parse --git-common-dir`, not only their worktree-specific git dir. The legacy `.git/refinedstone-credentials` name remains supported for existing checkouts.
- Configure repo-local Git credentials to use the selected file for push-capable commands when needed:
  `git config credential.helper ""`,
  `git config credential.username <github-login>`, and
  `git config --add credential.helper "store --file=$(git rev-parse --path-format=absolute --git-common-dir)/akra-github-credentials"`.
- If another `credential.helper` is inherited, override it in this repo's local `.git/config` only.
- Before the first push in an environment, verify `gh api user --jq .login` or `git credential fill` for the current `origin` resolves the intended GitHub login.
- Use `bash scripts/gh-akra.sh` for `pr create`, `pr view`, and review replies. Add `AKRA_GITHUB_LOGIN=<login>` or `--github-login <login>` when the operation must use a specific account.
- Do not use GitHub MCP tools for PR or review-thread writes unless their authenticated identity has been verified for the intended account.
- If a commit is created under the wrong author or committer identity, rewrite the branch history to the intended identity before any push or further review activity.
- If the intended identity cannot be verified, do not push, open PRs, or leave GitHub comments from that environment.

## Delivery Default

- Once a change reaches a reviewable milestone, the default is `commit -> push -> PR`.
- Do not stop at a local commit unless the user explicitly says to hold.
- After a PR merges or closes, start the next task from the latest target base branch on a new feature branch.
- For this repository's normal integration path, rebase the feature branch onto the latest `prerelease`, fast-forward or rebase-merge the reviewed commits into `prerelease`, push `prerelease`, then close the PR if the branch has already landed.
- After a PR is integrated into `prerelease`, return to the integration checkout and remove the finished feature worktree instead of leaving it parked indefinitely.
- Prefer `bash scripts/cleanup_merged_worktrees.sh --apply --branch <finished-branch>` for the lane you just integrated. The helper can also run without `--branch` as a conservative sweep, and it skips dirty or unmerged worktrees automatically.
- Do not use the cleanup helper for `akra-agent/slot-*` parallel-mode slot branches. Those slots carry runtime leases and must return through the parallel runtime cleanup path so the slot worktree, lease, and session detail stay consistent.
- If the finished lane is already integrated and its remaining worktree dirtiness is disposable repo noise, use `--force-dirty` only with an explicit `--branch` or `--path`. Do not use `--force-dirty` as a broad sweep.
- For final integration, do not use GitHub's merge-commit flow.
- Rebase locally, fast-forward the base branch with linear history, then close the PR after the base branch already contains the reviewed commits.

## Review Handling

- Inspect every new review comment and thread before changing code.
- Apply feedback only when it is logically correct and aligned with the chosen architecture and product direction.
- Fix correctness and low-cost maintainability issues that fit the current design.
- If a comment is wrong, stale, or pushes in the wrong direction, reply with a concise rationale instead of changing code.
- Reply on each review thread only when `bash scripts/gh-akra.sh auth status` can authenticate as the intended account; set `AKRA_GITHUB_LOGIN=<login>` when the identity must be pinned.
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
- Finished worktrees are not part of the live set. Once their branch is merged or closed, clean them up before opening more lanes so `git worktree list` remains actionable.
