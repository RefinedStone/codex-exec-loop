# Queue Idle Review Prompt

Use this prompt only when the executable queue is empty.

- Treat `docs/plan/14-product-elevation-blueprint.md`, `docs/plan/15-ux-flow-rearchitecture.md`, `docs/plan/16-planning-and-automation-evolution.md`, `docs/plan/17-structure-and-architecture-debt-map.md`, and `docs/plan/18-planning-workspace-lifecycle-commands.md` as the long-term product roadmap for this workspace.
- Treat `directions.toml` as the durable strategy map. Long-term intent belongs there; immediate execution slices belong in `task-ledger.json`.
- Assume this workspace is meant to sustain many hours of queue-driven improvement. Keep the long-term roadmap encoded in directions, but keep the executable queue limited to small, reviewable slices.
- Prefer active directions in this order unless current code state clearly blocks it:
  `workspace-lifecycle-commands`, `operator-state-language`, `queue-and-automation-trust`, `planning-authoring-ergonomics`, `session-continuity-and-recovery`, `architecture-boundaries`, `validation-and-release-confidence`.
- When the current product gap is about planning bootstrap, pre-launch inspection, or safe reset semantics, prefer `workspace-lifecycle-commands` even if another direction previously owned the top executable slice.
- Keep paused directions (`approval-review-operability`, `guided-planning-authoring`) out of the executable queue unless the operator explicitly reprioritizes them.
- Derive at most one `ready` or `in_progress` task when the next slice is concrete, reviewable on its own, and clearly moves the product toward the blueprint.
- Keep alternate or broader follow-up work as `proposed`.
- Prefer tasks that improve operator clarity, planning trust, or recovery safety before tasks that only add more hidden autonomy or broad new capability.
- When a direction still has substantial unfinished intent, derive the next sharp slice from its detail doc instead of creating one oversized umbrella task.
- Do not create queue work that only edits planning files unless planning maintenance itself is the explicit goal.
- If a refactor is justified, tie it to one operator-visible benefit in `direction_relation_note` and `description`.
- If no justified implementation slice exists, keep the queue empty and briefly explain why.
