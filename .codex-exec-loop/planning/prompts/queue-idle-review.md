# Queue Idle Review Prompt

Use this prompt only when the executable queue is empty.

- Treat `docs/supersession/README.md`, `docs/supersession/01-product-model.md`, `docs/supersession/02-operator-mode-and-shell-model.md`, `docs/supersession/04-task-ledger-feedback-loop.md`, `docs/supersession/05-git-worktree-pool.md`, `docs/supersession/06-distributor-and-merge-queue.md`, `docs/supersession/09-architecture-boundaries.md`, `docs/supersession/10-implementation-slices.md`, and the current direction detail docs as the long-term product roadmap for this workspace.
- Treat `directions.toml` as the durable strategy map. Long-term intent belongs there; immediate execution slices belong in `task-ledger.json`.
- Assume this workspace is meant to sustain many hours of queue-driven improvement. Keep the long-term roadmap encoded in directions, but keep the executable queue limited to small, reviewable slices.
- Prefer active directions in this order unless current code state clearly blocks it:
  `supersession-mode-entry-and-readiness`, `supersession-agent-control-tower`, `supersession-ledger-feedback-loop`, `supersession-git-worktree-pool`, `supersession-distributor-merge-queue`, `supersession-architecture-boundaries`.
- When the current product gap is about parallel-mode toggles, readiness states, degraded capability checks, or shell routing, prefer `supersession-mode-entry-and-readiness`.
- When the gap is about supervisor board layout, agent lifecycle visibility, selected-agent detail, or completion-feed clarity, prefer `supersession-agent-control-tower`.
- When the gap is about hidden planning worker refresh, official task authority, completion payloads, or preventing repeated queue-head assignment after agent work, prefer `supersession-ledger-feedback-loop`.
- When the gap is about slot reconcile, lease states, pool exhaustion, worktree-root collisions, or cleanup gates, prefer `supersession-git-worktree-pool`.
- When the gap is about merge queue sequencing, rebase policy, push and PR handling, or slot return after integration, prefer `supersession-distributor-merge-queue`.
- When the gap is about ports, services, runtime seams, or hotspot extraction that unblocks the other supersession directions, prefer `supersession-architecture-boundaries`.
- Derive at most one `ready` or `in_progress` task when the next slice is concrete, reviewable on its own, and clearly moves the product toward the blueprint.
- Keep alternate or broader follow-up work as `proposed`.
- Prefer tasks that make supersession mode entry, agent supervision, ledger authority, worktree hygiene, or distributor safety more concrete before broader capability expansion.
- When a direction still has substantial unfinished intent, derive the next sharp slice from its detail doc instead of creating one oversized umbrella task.
- Do not create queue work that only edits planning files unless planning maintenance itself is the explicit goal.
- If a refactor is justified, tie it to one operator-visible benefit in `direction_relation_note` and `description`.
- If no justified implementation slice exists, keep the queue empty and briefly explain why.
