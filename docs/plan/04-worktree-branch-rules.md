# Worktree Branch Rules

Use this file only for the current operating rules, not for sprint history.

## Base And Layout

- use `origin/prerelease` as the default base for feature, fix, docs, and chore work
- keep one integration checkout that owns local `prerelease`
- create live branches in sibling worktrees such as `../codex-exec-loop-worktrees/<branch-slug>`
- keep one branch, one reviewable slice, and one PR per worktree

Example:

```bash
git fetch origin
git worktree add ../codex-exec-loop-worktrees/feature-native-session-query-model \
  -b feature/native-session-query-model origin/prerelease
```

## Branch Names

- `feature/native-<lane>-<zone>-<slice>`
- `fix/native-<lane>-<zone>-<slice>`
- `docs/native-<lane>-<zone>-<slice>`
- `chore/native-<lane>-<zone>-<slice>`

Use a lane such as `shell`, `session`, `followup`, `planning`, `runtime`, `github`, or `platform`. Use a zone such as `runtime`, `presentation`, `overlay`, `adapter`, `service`, or `validation`.

## Preflight

Before opening a new lane:

- run `git worktree list`
- run `git branch -vv`
- inspect open PRs when available
- verify the repo still pushes as `RefinedStone` before the first push in an environment
- choose a disjoint lane or hotspot boundary when another unmerged branch already owns nearby files

If overlap is intentional, record the expected collision files in the task note or PR body.

## Hotspots

Avoid unrelated parallel edits in these files:

- `src/adapter/inbound/tui/app.rs`
- `src/adapter/inbound/tui/app/app_runtime.rs`
- `src/adapter/inbound/tui/app/shell_rendering.rs`
- `src/adapter/inbound/tui/app/shell_presentation.rs`
- `src/adapter/inbound/tui/app/shell_controller.rs`
- `src/adapter/inbound/tui/app/planning_init_overlay_ui.rs`
- `src/adapter/inbound/tui/app/planning_draft_editor_ui.rs`
- `src/application/service/planning_init_service.rs`
- `src/application/service/planning_prompt_service.rs`
- `src/application/service/planning_reconciliation_service.rs`
- `docs/README.md`
- `docs/design/06-planning-runtime-and-draft-editor.md`
- `docs/plan/10-inline-scrollback-shell.md`

## Review And Merge

- keep the slice small enough to be reviewable on its own
- rebase worktree branches with `git fetch origin && git rebase origin/prerelease`
- keep exactly one PR tied to the branch
- if the branch already has a PR, push rebases with `--force-with-lease`
- do not use GitHub merge commits for final integration
- fast-forward `prerelease` from the integration checkout after the reviewed branch head is ready

## Cleanup

Prefer the cleanup helper from the integration checkout after a branch lands in `prerelease`:

```bash
bash scripts/cleanup_merged_worktrees.sh --apply --branch feature/native-session-query-model
```

The helper removes a targeted finished branch immediately, or can scan for conservative merged candidates when run without `--branch`. It removes only non-root worktrees whose workspace is clean, and the scan mode skips dirty or still-active lanes automatically.

If a finished lane is already integrated but still reports disposable worktree dirtiness, use the explicit force form for that one lane only:

```bash
bash scripts/cleanup_merged_worktrees.sh --apply --branch feature/native-session-query-model --force-dirty
```

If a branch is abandoned without merge, use the manual fallback for that specific lane.

Manual fallback:

```bash
git worktree remove ../codex-exec-loop-worktrees/feature-native-session-query-model
git branch -d feature/native-session-query-model
git push origin --delete feature/native-session-query-model
```
