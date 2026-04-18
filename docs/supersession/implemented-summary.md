# Implemented Supersession Summary

This file merges the supersession design topics that are already implemented on the current branch
or already shipped on `origin/prerelease`.

## Current Truth

- [../design/01-current-product-state.md](../design/01-current-product-state.md)
- [../design/02-tui-shell-flow.md](../design/02-tui-shell-flow.md)
- [../design/06-planning-runtime-and-draft-editor.md](../design/06-planning-runtime-and-draft-editor.md)
- [../releases/v1.2.9-to-prerelease.md](../releases/v1.2.9-to-prerelease.md)

## Implemented Areas

| Area | Status | Primary current truth |
| --- | --- | --- |
| mode entry and readiness | implemented | `docs/design/01-current-product-state.md` |
| supervisor board and agent control tower | implemented | `docs/design/01-current-product-state.md`, `docs/design/02-tui-shell-flow.md` |
| reported versus official completion | implemented | `docs/design/01-current-product-state.md`, `docs/design/06-planning-runtime-and-draft-editor.md` |
| worktree pool and distributor flow | implemented | `docs/design/01-current-product-state.md` |
| repo-scoped planning authority store | implemented on current branch | `docs/design/06-planning-runtime-and-draft-editor.md` |
| restart recovery and export repair | implemented on current branch | `docs/design/01-current-product-state.md`, `docs/design/06-planning-runtime-and-draft-editor.md` |

## Merged Topics

The older detailed notes for these topics were removed during docs diet because they mostly
repeated implemented contracts:

- product model
- operator mode and shell model
- agent session lifecycle
- task-ledger feedback loop
- git worktree pool
- distributor and merge queue
- supervisor UI and surfaces
- capabilities, degraded mode, and failures
- architecture boundaries

## Still Detailed Elsewhere

- remaining validation, docs alignment, and polish: [10-implementation-slices.md](10-implementation-slices.md)
- still-open questions and explicit non-goals: [11-open-questions-and-non-goals.md](11-open-questions-and-non-goals.md)
- historical redesign and audit detail: `docs/plan/18-*` and `docs/plan/19-*`
