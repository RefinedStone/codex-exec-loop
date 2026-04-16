# Queue Idle Review Prompt

Use this prompt only when the executable queue is empty.

- Treat `docs/plan/14-product-elevation-blueprint.md`, `docs/plan/15-ux-flow-rearchitecture.md`, `docs/plan/16-planning-and-automation-evolution.md`, `docs/plan/17-structure-and-architecture-debt-map.md`, and the current direction detail docs as the long-term product roadmap for this workspace.
- Treat `directions.toml` as the durable strategy map. Long-term intent belongs there; immediate execution slices belong in `task-ledger.json`.
- Assume this workspace is meant to sustain many hours of queue-driven improvement. Keep the long-term roadmap encoded in directions, but keep the executable queue limited to small, reviewable slices.
- Prefer active directions in this order unless current code state clearly blocks it:
  `operator-state-language`, `queue-workboard-projection`, `automation-recovery-language`, `planning-entry-ergonomics`, `architecture-boundaries`.
- When the current product gap is about shell wording, pause explanation, resumed-session language, or shared status projection, prefer `operator-state-language`.
- When the gap is about queue inspection layout or blocked-work visibility, prefer `queue-workboard-projection`.
- When the gap is about pause reasons, stop reasons, or recovery guidance in the automation surface, prefer `automation-recovery-language`.
- When the gap is about simple mode, first-run planning review, or keeping disabled guided branches out of the main path, prefer `planning-entry-ergonomics`.
- When the gap is about hotspot extraction, prefer `architecture-boundaries` only if the extracted seam clearly unlocks one operator-visible improvement.
- Derive at most one `ready` or `in_progress` task when the next slice is concrete, reviewable on its own, and clearly moves the product toward the blueprint.
- Keep alternate or broader follow-up work as `proposed`.
- Prefer tasks that improve operator clarity, queue legibility, recovery guidance, or planning entry simplicity before tasks that only add more hidden autonomy or broad new capability.
- When a direction still has substantial unfinished intent, derive the next sharp slice from its detail doc instead of creating one oversized umbrella task.
- Do not create queue work that only edits planning files unless planning maintenance itself is the explicit goal.
- If a refactor is justified, tie it to one operator-visible benefit in `direction_relation_note` and `description`.
- If no justified implementation slice exists, keep the queue empty and briefly explain why.
