# Task Ledger Feedback Loop

This document defines the target supersession model, not shipped behavior.

## Authority Rule

`.codex-exec-loop/planning/task-ledger.json` remains the official source of executable task state.

That means:

- agent sessions never mutate the ledger directly
- supervisor never treats an agent report as official task completion on its own
- new assignment decisions happen only after ledger refresh

## Completion Feedback Flow

1. agent reaches `reported_complete`
2. supervisor captures the completion report and stores it as non-official event data
3. supervisor invokes hidden planning worker with the completion payload
4. hidden planning worker refreshes `task-ledger.json`
5. supervisor reloads planning runtime snapshot
6. if refresh succeeds, agent transitions to `commit_ready`
7. if refresh fails, agent transitions to `failed`

## Required Completion Payload

The hidden planning worker refresh call must include:

| Field | Purpose |
| --- | --- |
| `agent_id` | identifies which agent produced the result |
| `task_id` | links the result to the assigned ledger task |
| `task_title` | gives planning worker stable human-readable context |
| `branch_name` | lets planning and distributor describe the result provenance |
| `worktree_path` | identifies where the execution happened |
| `commit_sha` | marks the commit-ready boundary |
| `validation_summary` | records whether required checks passed, failed, or were skipped |
| `final_response_summary` | concise implementation outcome |
| `final_response_text` | optional long-form completion text when needed for follow-up derivation |
| `failure_context` | optional explanation when execution stopped in partial or blocked state |
| `completed_at` | timestamp for ordering and repeat prevention |

## Official State After Refresh

The ledger refresh is responsible for making the official decision about:

- whether the assigned task becomes `done`
- whether it becomes `blocked` or stays active with updated wording
- whether follow-up tasks become `ready`
- which candidates remain `proposed`
- whether the queue is effectively idle after the result

Supervisor does not invent new task state outside this refresh.

## Serialization Rules

Multiple agent completions may arrive close together. Hidden planning refresh must therefore be
serialized.

Closed v1 rules:

- only one ledger refresh may run at a time
- refresh order follows completion event order
- later refreshes always read the ledger produced by earlier refreshes
- assignment cannot consume newly ready work until the current refresh finishes

## Repeat Prevention

The system must avoid reassigning the same unchanged queue head after an agent result. The planning
refresh payload must include enough provenance for hidden planning worker to:

- detect that the current completion belongs to the previously assigned task
- avoid re-promoting the same unchanged task as queue head
- mark the task `done` or update it materially before returning it to ready state

## Failure Handling

### Planning Refresh Failure

- keep the agent in `failed`
- do not enqueue distributor integration
- preserve the agent completion report for operator inspection
- keep the slot reserved until the operator resolves the planning issue

### Planning Workspace Invalid

- parallel mode may remain open, but no new assignments occur
- supersession surface must explain that ledger authority is unavailable

## Assignment Rule After Refresh

After a successful refresh:

- re-read official queue state from the ledger
- assign only ledger-ready tasks
- never assign directly from raw agent text

## Related Docs

- [03-agent-session-lifecycle.md](03-agent-session-lifecycle.md)
- [06-distributor-and-merge-queue.md](06-distributor-and-merge-queue.md)
- [../design/06-planning-runtime-and-draft-editor.md](../design/06-planning-runtime-and-draft-editor.md)

## Code Impact

Expected entrypoints:

- `src/application/service/planning_worker_orchestration_service.rs`
- `src/application/service/planning_runtime_facade_service.rs`
- `src/application/service/planning_runtime_policy_service.rs`
