# Planning Worker Session Orchestration

## Goal

Keep the main user-facing Codex session focused on operator conversation and execution work.

Planning queue maintenance should not pollute the main thread context with:

- `task-ledger.json` rules
- `queue.snapshot.json` semantics
- planning repair prompts
- queue refresh instructions

Instead, the host app should delegate planning mutations to a fresh hidden planner session.

## Scope

This v1 change applies to `builtin next-task` auto follow-up flow.

It also moves planning repair prompts off the main session and into hidden planner sessions.

## Previous Flow

Before this change:

- the main session received planning prompt fragments during manual turns
- `builtin next-task` could ask the main session to refresh the planning queue itself
- invalid planning candidates could trigger repair prompts in the main session
- the conversation transcript mixed user work and planning maintenance

## New Flow

### Main session

The main session is now treated as the user-facing execution session.

It:

- receives the operator prompt without injected planning fragments
- produces the normal turn response
- receives a natural-language next-task handoff when `builtin next-task` auto follow-up is ready

It does **not** receive raw planning refresh or repair prompts.

### Planner worker session

Planning mutations run through a hidden planner worker.

Characteristics:

- implemented with `codex app-server`
- each planning operation uses a **fresh thread**
- planner threads are never resumed or reused
- planner transcripts are not mixed into the main conversation transcript

Operations:

- refresh queue from the latest main-session reply
- repair an invalid `task-ledger.json` candidate

### Host responsibilities

The host keeps orchestration and validation responsibilities:

- capture accepted planning snapshot before worker execution
- invoke planner worker
- reconcile protected planning files after worker execution
- accept or reject `task-ledger.json`
- rebuild `queue.snapshot.json` from accepted planning state
- render planner status in the TUI
- hand the main session a natural-language next-task prompt

## Boundaries

### Planner-owned artifact

- `task-ledger.json` candidate

### Host-owned artifacts

- `queue.snapshot.json`
- protected planning file restoration
- validation and reconciliation decisions
- next-task handoff prompt shown to the main session

The planner worker is instructed not to edit:

- `directions.toml`
- `task-ledger.schema.json`
- `result-output.md`
- `queue.snapshot.json`

## Builtin Next-Task Sequence

1. Main session finishes a normal turn.
2. Host reconciles any planning files the main session changed.
3. If reconciliation produced an invalid `task-ledger.json` candidate, host runs hidden repair attempts.
4. If the selected auto-follow template is `builtin next-task`, host starts a fresh planner worker thread.
5. Planner worker refreshes `task-ledger.json` from the latest main-session reply.
6. Host reconciles and validates the worker output.
7. If needed, host runs hidden repair attempts for the planner worker candidate.
8. Host rebuilds `queue.snapshot.json` from the accepted ledger.
9. Host converts the queue head into a natural-language handoff prompt.
10. Main session receives that handoff as the auto-follow submission.

## Failure Handling

### Main-session planning corruption

If the main session changes planning files incorrectly:

- host restores protected files from the accepted snapshot
- host archives rejected planning candidates when appropriate
- host may run hidden planner repair attempts

### Planner refresh failure

If planner refresh cannot produce an accepted ledger:

- host keeps the last accepted planning state on disk
- host marks the runtime snapshot as blocked for `builtin next-task`
- auto follow-up pauses instead of sending a stale queue head into the main session

## TUI Surface

Planner session transcript stays hidden.

The UI exposes planner state through planner status lines in the follow-up/status surfaces:

- planner status
- last queue summary
- last planner detail summary
- last rejected planning summary

This keeps planner activity visible without polluting the main transcript.

## Implementation Notes

Key types added or changed:

- `PlanningWorkerPort`
- `AppServerPlanningWorkerAdapter`
- `PlanningWorkerOrchestrationService`
- `PlanningRuntimeFacadeService::build_builtin_next_task_handoff`

Key behavior changes:

- manual main-session prompts no longer append planning context
- `builtin next-task` no longer queues a planning-refresh prompt into the main session
- planning repair is hidden from the main transcript

