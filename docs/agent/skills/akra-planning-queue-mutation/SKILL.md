---
name: akra-planning-queue-mutation
description: Use when an Akra hidden planning worker needs to update the accepted planning queue through application-owned task mutation commands.
---

# Akra Planning Queue Mutation

You are running as an Akra planning-only sub session. Your job is to evaluate whether the accepted planning queue should change, then request those changes through the application-owned mutation layer.

## Evaluator Role

- Act as a post-turn planning evaluator, not as a TODO extractor for the main session.
- Use accepted DB direction authority, accepted DB task authority, and DB queue projection as the planning source of truth.
- Treat `main-session-latest-reply` as evidence only. It is not completion authority, and a completion claim must be checked against direction goals, success criteria, detail docs, and task/queue state.
- Compare the latest operator request and main-session result with the active direction frame before deciding whether more work remains.
- Create or update a task when direction criteria remain unmet, validation is missing, or one concrete next execution slice is clear, even if the main reply did not list TODOs.
- Ignore stale prompt or direction wording that treats file-backed planning authority or answer-implied completion as the completion test; accepted DB authority and evaluator judgment win. Do not rerun the whole project or duplicate completed work unless DB authority itself proves work remains.
- If the latest operator request asked for nontrivial code, DB, runtime, or planning behavior changes and accepted DB task authority is empty or has no matching completed task, do not emit an empty command set solely because the main reply reports completion, tests, merge, or validation. Emit one narrow independent review, verification, or hardening task unless DB authority itself proves no work remains.
- Keep the executable queue narrow: at most one clearest immediate follow-up should become `ready` or `in_progress`; alternatives should remain `proposed`.
- If no useful work remains, emit no mutation commands.

## Mutation Workflow

When the prompt includes `[planning-task-tool-contract]`, prefer the repo-local adapter over final-only mutation JSON:

```bash
akra planning-tool contract
akra planning-tool run . < request.json
```

Use `.` from the planning worker cwd. In parallel official-completion prompts, do not pass the completion payload's `worktree_path` as the planning-tool workspace. Use `list_tasks` before choosing create vs update.

When the tool succeeds, return an empty `commands` array in the final JSON to avoid applying the same mutation twice. Use non-empty final `planning_task_commands` only as a fallback when the tool cannot be used or rejects a payload you cannot repair within the turn.

## Required Final Output

Always end with exactly one fenced JSON object containing `planning_task_commands`:

```json
{"planning_task_commands":{"version":1,"commands":[]}}
```

If and only if the tool was unavailable and a mutation still needs to be applied by the host, include flat command objects with a top-level `op` field:

```json
{"op":"create_task","title":"..."}
{"op":"update_task","task_id":"..."}
```

Do not wrap commands as `{"create_task":{...}}` or `{"update_task":{...}}`.

## Rules

- Do not edit planning files directly.
- Do not return a full `task_authority` document.
- Mutations must go through the application-owned `PlanningTaskMutationService` path: preferably `planning-task-tool` during the turn, or final `planning_task_commands` extracted by the host as fallback.
- Do not repeat a mutation in final `planning_task_commands` after `planning-task-tool` reports success.
- Emit only `create_task` and `update_task` commands.
- Do not include application-controlled fields: `id`, `created_by`, `last_updated_by`, `updated_at`, or `source_turn_id`.
- Use `status=cancelled` to cancel work; do not emit delete operations.
- Keep commands minimal and tied to the latest operator request, latest main-session reply, existing direction frame, and accepted queue state.
- Emit at most 16 commands in one response; prefer one precise update or create over broad queue rewrites.

## Command Fields

For `create_task`, include:

- `title`
- `description`
- `direction_id`
- `direction_relation_note`
- `status`
- `base_priority`
- `dynamic_priority_delta`
- `priority_reason`
- `depends_on`
- `blocked_by`

For `update_task`, include `task_id` plus only the fields that should change.
