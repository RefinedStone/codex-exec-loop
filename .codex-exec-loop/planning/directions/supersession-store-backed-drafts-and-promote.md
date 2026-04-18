# Supersession store-backed drafts and promote

## Outcome

Move draft storage, validation, rejection resume metadata, and promote semantics into the
repo-shared authority domain while leaving active planning unchanged until promote succeeds.

## Status

- Current branch status: implemented and recorded as `done` in `task-ledger.json`.
- Keep this file as compact rationale for what shipped; the remaining work moved on to validation, doc alignment, and residual polish rather than another draft-migration slice.

## Why this direction exists

The updated authority design keeps the operator mental model of `draft -> validate -> promote`, but
it no longer allows worktree-local draft files to act as the real source of pending changes. Drafts
must become durable repo-scoped state before store-primary can exist.

## Long-horizon plan

- store draft planning state in the authority domain
- validate draft content against the same rules as active planning
- preserve rejection archives and resume-able draft context
- commit promote as the only path that can replace or merge active planning

## Near-term bias

- start with draft data-model parity and validation behavior
- keep structured change requests mutating draft state by default
- prove active planning and queue projection do not move until promote commits

## Relevant inputs

- `docs/plan/18-repo-shared-planning-authority-store.md`
- `docs/supersession/09-architecture-boundaries.md`
- `docs/supersession/10-implementation-slices.md`
- `docs/design/06-planning-runtime-and-draft-editor.md`

## Task derivation guidance

- derive one draft-state capability at a time: load, edit, validate, reject, resume, promote
- keep promote atomicity explicit in naming and tests
- reuse current draft UX expectations instead of redefining operator flow

## Avoid

- direct active-authority mutation as the default authoring path
- dropping rejection-resume behavior as a temporary simplification
