# Supersession Docs

Use this folder for two things only:

- remaining supersession follow-through that is not fully validated or polished yet
- merged historical notes for implemented supersession contracts

Current behavior should be read from `docs/design/` first.

## Current Truth

- [../design/01-current-product-state.md](../design/01-current-product-state.md)
- [../design/02-tui-shell-flow.md](../design/02-tui-shell-flow.md)
- [../design/06-planning-runtime-and-draft-editor.md](../design/06-planning-runtime-and-draft-editor.md)
- [../releases/v1.2.9-to-prerelease.md](../releases/v1.2.9-to-prerelease.md)

## In This Folder

- [implemented-summary.md](implemented-summary.md): merged summary of supersession areas that are already implemented on the current branch
- [10-implementation-slices.md](10-implementation-slices.md): remaining validation, docs alignment, and surface polish work
- [11-open-questions-and-non-goals.md](11-open-questions-and-non-goals.md): still-open questions and explicit non-goals

## Current Status

- `origin/prerelease` already ships the first operator-facing supersession loop.
- The current branch also includes the repo-scoped planning authority follow-through.
- Most earlier supersession design notes have been merged into [implemented-summary.md](implemented-summary.md) to avoid repeating shipped contracts across many files.

## Working Rule

- Keep implemented behavior compact here and detailed in `docs/design/`.
- Keep only genuinely unfinished or still-open supersession material detailed here.
