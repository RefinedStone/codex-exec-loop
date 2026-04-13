# Planning Runtime And Draft Editor

This file describes the planning feature that already ships on `prerelease`.

## Active Files

- `.codex-exec-loop/planning/directions.toml`: operator-owned direction catalog
- `.codex-exec-loop/planning/directions/<direction-id>.md`: optional operator-owned direction detail docs
- `.codex-exec-loop/planning/task-ledger.json`: accepted task ledger
- `.codex-exec-loop/planning/task-ledger.schema.json`: schema used for validation and repair
- `.codex-exec-loop/planning/result-output.md`: operator-owned result-output prompt fragment
- `.codex-exec-loop/planning/prompts/queue-idle-review.md`: optional queue-idle review prompt
- `.codex-exec-loop/planning/queue.snapshot.json`: runtime-derived queue snapshot
- `.codex-exec-loop/planning/drafts/<draft>/...`: staged draft workspace
- `.codex-exec-loop/planning/rejected/<turn>/...`: archived rejected task-ledger candidates

## Operator Entry

- `:planning` opens planning mode inside the shell
- `:directions` opens staged directions maintenance inside the shell
- `simple mode` stages one generic active direction, an empty task ledger, and the default queue-idle review prompt
- `detail mode -> manual` opens the embedded draft editor for richer authoring
- `detail mode -> llm-assisted` is shown in the UI but currently disabled
- staged drafts stay inactive until the operator explicitly promotes them

## Runtime Contract

- manual submit and auto follow-up both append the same planning prompt fragment when planning files are valid
- planning runtime state is operator-visible as `uninitialized`, `invalid`, `ready with no task`, or `ready with task`
- `queue.snapshot.json` is derived from the accepted direction catalog and task ledger; it is never operator-authoritative
- `proposed` tasks are visible as proposal candidates and stay out of the normal executable queue
- each direction can optionally point at a detail markdown doc via `detail_doc_path`
- queue-idle behavior is explicit in `directions.toml` under `[queue_idle]`

## Reconciliation And Repair

- `directions.toml`, `task-ledger.schema.json`, `result-output.md`, and `queue.snapshot.json` are protected during active automated execution
- if the LLM changes a protected file, the runtime restores the pre-turn snapshot and can archive the rejected candidate
- `task-ledger.json` becomes authoritative only after schema validation and business-rule validation both pass
- invalid task-ledger writes are rolled back, archived under `rejected/`, and followed by a bounded repair retry prompt
- builtin `next-task` uses the accepted queue head when one exists
- if planning is valid but there is no actionable head, the runtime follows `queue_idle.policy`
- `queue_idle.policy = stop` ends post-turn automation cleanly
- `queue_idle.policy = review_and_enqueue` starts a hidden queue-manager worker that may add justified ready/proposed tasks without mutating operator-owned files

## Draft Editor Contract

- the embedded editor works on staged draft files inside the shell
- `Ctrl+S` saves and re-validates the staged draft
- `Ctrl+P` saves and promotes the staged files into the active planning workspace when valid
- close is guarded when staged buffers are dirty or the staged draft is invalid
- the shell surfaces queue summary, proposal summary, and latest planning failure without leaving the conversation flow
- `:directions` uses the same staged editor mechanics but narrows editable files to `directions.toml` plus selected detail/prompt markdown files
