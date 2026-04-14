# Planning Runtime And Draft Editor

This file records the active planning contract.

## Workspace Files

- `.codex-exec-loop/planning/directions.toml`
- `.codex-exec-loop/planning/directions/<direction-id>.md`
- `.codex-exec-loop/planning/task-ledger.json`
- `.codex-exec-loop/planning/task-ledger.schema.json`
- `.codex-exec-loop/planning/result-output.md`
- `.codex-exec-loop/planning/prompts/queue-idle-review.md`
- `.codex-exec-loop/planning/queue.snapshot.json`
- `.codex-exec-loop/planning/drafts/<draft>/...`
- `.codex-exec-loop/planning/rejected/<turn>/...`

## Operator Entry

- `:planning` opens planning workspace controls.
- `:planning on|off` toggles plan execution without deleting the workspace.
- `:directions` opens directions maintenance.
- Simple mode stages a minimal planning workspace and can promote immediately or open the draft editor.
- Detail mode opens the embedded draft editor.
- Staged drafts stay inactive until explicit promotion.

## Runtime Contract

- Manual submit and auto follow-up both append the same accepted planning prompt fragment.
- Runtime state is surfaced as uninitialized, invalid, ready without task, or ready with task.
- `queue.snapshot.json` is derived state only.
- Proposed tasks do not enter the executable queue until promoted.
- Queue-idle behavior is driven by `[queue_idle]` in `directions.toml`.

## Reconciliation And Worker Rules

- `directions.toml`, `task-ledger.schema.json`, `result-output.md`, and `queue.snapshot.json` are protected during automated execution.
- Invalid `task-ledger.json` writes are rolled back, archived, and may trigger a bounded repair retry.
- Queue refresh and repair work run through the planning worker boundary.
- Builtin `next-task` uses the accepted queue head only.
- If the queue is valid but idle, runtime behavior follows `queue_idle.policy`.

## Code Entry

- Application entrypoint: `src/application/service/planning`
- TUI entrypoint: `src/adapter/inbound/tui/app/planning`
