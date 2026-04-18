# Queue Idle Review Prompt

Use this prompt only when the executable queue is empty.

- Treat `docs/design/01-current-product-state.md`, `docs/design/06-planning-runtime-and-draft-editor.md`, `docs/supersession/README.md`, `docs/supersession/implemented-summary.md`, `docs/supersession/10-implementation-slices.md`, `docs/supersession/11-open-questions-and-non-goals.md`, and the current direction detail docs as the roadmap context for this workspace.
- Treat `directions.toml` as the durable strategy map. Long-term intent belongs there; immediate execution slices belong in `task-ledger.json`.
- Assume this workspace is meant to sustain many hours of queue-driven improvement. Keep the long-term roadmap encoded in directions, but keep the executable queue limited to small, reviewable slices.
- Treat completed supersession authority-store slices as historical context, not as a cue to recreate already-finished architecture tasks.
- Prefer follow-up work only when current code or validation still shows a concrete gap in validation coverage, docs alignment, operator copy, or remaining open questions.
- Derive at most one `ready` or `in_progress` task when the next slice is concrete, reviewable on its own, and clearly moves the product toward the blueprint.
- Keep alternate or broader follow-up work as `proposed`.
- When all supersession directions are already `done`, prefer validation, docs, or operator-surface polish tasks over faux architecture revival.
- Do not create queue work that only edits planning files unless planning maintenance itself is the explicit goal.
- If a refactor is justified, tie it to one operator-visible benefit in `direction_relation_note` and `description`.
- If no justified implementation slice exists, keep the queue empty and briefly explain why.
