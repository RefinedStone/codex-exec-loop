# Supersession authority locator and shadow store

## Outcome

Replace worktree-local planning roots with one canonical repo-scoped authority location and a
shadow store that can mirror tracked planning state without taking runtime authority yet.

## Status

- Current branch status: implemented and recorded as `done` in `task-ledger.json`.
- Keep this file as compact rationale for what shipped; the remaining work moved on to validation, doc alignment, and residual polish rather than another authority-locator slice.

## Why this direction exists

The updated supersession design rejects worktree-local planning authority. Every leased worktree in
the same repo family must resolve one authority root before store-backed drafts, claims, or runtime
recovery can become safe.

## Long-horizon plan

- resolve any active workspace path to one canonical repo authority root
- bootstrap the repo-shared authority store and schema
- mirror tracked planning files into the store in `shadow-store` mode
- expose parity diagnostics before runtime reads or writes move to the store

## Near-term bias

- land canonical authority location and read-only mirroring before active mutation features
- keep tracked planning files authoritative in this slice
- prefer cross-worktree parity proof over authoring ergonomics

## Relevant inputs

- `docs/plan/18-repo-shared-planning-authority-store.md`
- `docs/plan/19-supersession-runtime-risk-audit.md`
- `docs/supersession/09-architecture-boundaries.md`
- `docs/supersession/10-implementation-slices.md`
- `docs/design/06-planning-runtime-and-draft-editor.md`

## Task derivation guidance

- derive work around one repo-scope locator and mirror capability at a time
- keep parity checks explicit in naming and tests
- prove same-repo worktrees resolve one authority root before adding mutations

## Avoid

- treating the store as runtime authority before parity is proven
- leaving worktree-local planning roots implicit anywhere in new code
