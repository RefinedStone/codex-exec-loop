# Queue Idle Review Prompt

Use this prompt only when the executable queue is empty.

- Treat `docs/supersession/README.md`, `docs/supersession/01-product-model.md`, `docs/supersession/02-operator-mode-and-shell-model.md`, `docs/supersession/04-task-ledger-feedback-loop.md`, `docs/supersession/05-git-worktree-pool.md`, `docs/supersession/06-distributor-and-merge-queue.md`, `docs/supersession/09-architecture-boundaries.md`, `docs/supersession/10-implementation-slices.md`, and the current direction detail docs as the long-term product roadmap for this workspace.
- Treat `directions.toml` as the durable strategy map. Long-term intent belongs there; immediate execution slices belong in `task-ledger.json`.
- Assume this workspace is meant to sustain many hours of queue-driven improvement. Keep the long-term roadmap encoded in directions, but keep the executable queue limited to small, reviewable slices.
- Prefer active directions in this order unless current code state clearly blocks it:
  `supersession-authority-locator-and-shadow-store`, `supersession-store-backed-drafts-and-promote`, `supersession-active-planning-mutation-and-queue-claims`, `supersession-runtime-projections-and-recovery`, `supersession-store-primary-cutover`.
- When the gap is about canonical repo authority roots, worktree-family planning consistency, store bootstrap, or parity diagnostics, prefer `supersession-authority-locator-and-shadow-store`.
- When the gap is about draft storage, validation parity, rejection resume, or keeping active planning unchanged until promote succeeds, prefer `supersession-store-backed-drafts-and-promote`.
- When the gap is about hidden planning refresh commits, queue projection transactions, official refresh reservation, or queue-head claim uniqueness, prefer `supersession-active-planning-mutation-and-queue-claims`.
- When the gap is about durable slot/session/distributor projections, restart recovery, orphaned claims, or Git and GitHub truth rechecks, prefer `supersession-runtime-projections-and-recovery`.
- When the gap is about store-primary mode, revision-stamped export artifacts, explicit tracked-file import, or removing implicit file authority, prefer `supersession-store-primary-cutover`.
- Derive at most one `ready` or `in_progress` task when the next slice is concrete, reviewable on its own, and clearly moves the product toward the blueprint.
- Keep alternate or broader follow-up work as `proposed`.
- Prefer tasks that make repo-shared planning authority, draft safety, claim uniqueness, recovery safety, or store-primary cutover more concrete before broader capability expansion.
- When a direction still has substantial unfinished intent, derive the next sharp slice from its detail doc instead of creating one oversized umbrella task.
- Do not create queue work that only edits planning files unless planning maintenance itself is the explicit goal.
- If a refactor is justified, tie it to one operator-visible benefit in `direction_relation_note` and `description`.
- If no justified implementation slice exists, keep the queue empty and briefly explain why.
