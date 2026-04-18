# Supersession store-primary cutover

## Outcome

Make the repo-shared authority store the default active and draft planning authority while tracked
planning files become revision-stamped exports and explicit imports only.

## Status

- Current branch status: implemented and recorded as `done` in `task-ledger.json`.
- Keep this file as compact rationale for what shipped; the remaining work moved on to validation, doc alignment, and residual polish rather than another store-primary cutover slice.

## Why this direction exists

The earlier slices only harden the architecture if runtime authority actually moves. This cutover
must end dual-authority ambiguity without giving up reviewability or portability for planning
artifacts.

## Long-horizon plan

- switch runtime reads and writes to store-backed active and draft state
- export tracked planning artifacts with revision metadata
- allow tracked-file import only through explicit operator flow
- preserve review and portability workflows without restoring file authority

## Near-term bias

- start with gated mode switch and revision-stamped export surfaces
- prefer one-way mirror rules over convenience sync
- keep import guarded by explicit operator action and base revision checks

## Relevant inputs

- `docs/plan/18-repo-shared-planning-authority-store.md`
- `docs/plan/19-supersession-runtime-risk-audit.md`
- `docs/supersession/09-architecture-boundaries.md`
- `docs/supersession/10-implementation-slices.md`
- `docs/design/06-planning-runtime-and-draft-editor.md`

## Task derivation guidance

- derive one cutover concern at a time: mode switch, export, import, operator review flow
- keep parity and base-revision safety checks visible in naming and tests
- treat tracked planning files as review artifacts even when they remain branch-visible

## Avoid

- automatic two-way sync between files and the store
- hidden fallback from store-primary back to implicit file authority
