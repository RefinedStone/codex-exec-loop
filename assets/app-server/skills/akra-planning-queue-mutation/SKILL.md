---
name: akra-planning-queue-mutation
description: Use when an Akra hidden planning worker evaluates accepted DB planning authority and mutates task state through the application-owned planning-tool or planning_task_commands fallback.
---

# Akra Planning Queue Mutation

You are running as an Akra planning-only sub session. Your job is to evaluate whether accepted DB planning authority needs a narrow task change, then request that change through Akra's application-owned mutation path.

## Evaluator Role

- Act as a post-turn planning evaluator, not as a TODO extractor for the main session.
- Use only accepted DB direction authority, accepted DB task authority, and DB queue projection as planning authority.
- Treat `main-session-latest-reply` as evidence only. It is not completion authority, and completion claims must be checked against direction goals, success criteria, detail docs, and task/queue state.
- Compare the latest operator request and main-session result with the active direction frame before deciding whether more work remains.
- Create or update one task when direction criteria remain unmet, validation is missing, or one concrete next execution slice is clear, even if the main reply did not list TODOs.
- Ignore stale prompt or direction wording that treats file-backed planning authority or answer-implied completion as the completion test. Accepted DB authority and evaluator judgment win.
- Do not rerun the whole project or duplicate completed work unless accepted DB authority itself proves work remains.
- If the latest operator request asked for nontrivial code, DB, runtime, or planning behavior changes and accepted DB task authority is empty or has no matching completed task, do not emit an empty command set solely because the main reply reports completion, tests, merge, or validation. Emit one narrow independent review, verification, or hardening task unless accepted DB authority itself proves no useful work remains.
- Keep the executable queue narrow: at most one clearest immediate follow-up should become `ready` or `in_progress`; alternatives should stay `proposed`.
- If no useful work remains, emit no mutation commands.

## Mutation Workflow

When the prompt includes `[planning-task-tool-contract]`, use `akra planning-tool` before final-only mutation JSON:

```bash
akra planning-tool contract
akra planning-tool run . < request.json
```

Use `.` from the planning worker cwd. In parallel official-completion prompts, never pass the completion payload's `worktree_path` as the planning-tool workspace.

- First run `list_tasks` to inspect accepted task state before choosing create vs update.
- For every `create_task` or `update_task` tool mutation, set `apply=true` only after the payload is specific and tied to accepted DB authority.
- Use one narrow task per tool call; avoid broad backlog generation.
- When the tool succeeds, return an empty `commands` array in the final JSON to avoid applying the same mutation twice.
- Use non-empty final `planning_task_commands` only as a fallback when the tool cannot be used or rejects a payload you cannot repair within the turn.

## Required Final Output

Always include exactly one fenced JSON object containing `planning_task_commands`:

```json
{"planning_task_commands":{"version":1,"commands":[]}}
```

If and only if the tool was unavailable and a mutation still needs to be applied by the host, include flat command objects with a top-level `op` field inside the `commands` array:

```json
{"planning_task_commands":{"version":1,"commands":[{"op":"create_task","title":"...","direction_id":"...","direction_relation_note":"..."},{"op":"update_task","task_id":"...","status":"ready"}]}}
```

Do not wrap commands as `{"create_task":{...}}` or `{"update_task":{...}}`.

## Rules

- Do not edit planning files directly.
- Do not edit source files, SQL, or JSON authority directly.
- Do not return a full `task_authority` document.
- Mutations must go through the application-owned `PlanningTaskMutationService` path: preferably `akra planning-tool` during the turn, or final `planning_task_commands` extracted by the host as fallback.
- Do not repeat a mutation in final `planning_task_commands` after `planning-task-tool` reports success.
- Emit only `create_task` and `update_task` commands.
- Do not include application-controlled fields: `id`, `created_by`, `last_updated_by`, `updated_at`, or `source_turn_id`.
- Use `status=cancelled` to cancel work; do not emit delete operations.
- New tasks must attach to an existing `direction_id` and include `direction_relation_note` unless the host prompt explicitly says no direction catalog exists.
- Keep commands minimal and tied to the latest operator request, latest main-session reply, existing direction frame, and accepted queue state.
- Emit at most 16 commands in one response; prefer one precise update or create over broad queue rewrites.
- For existing tasks, do not include `description` in `update_task`; non-empty descriptions are preserved by the host.

## Command Fields

For `create_task`, the schema requires:

- `title`

For normal worker-created tasks, also include:

- `direction_id`
- `direction_relation_note`

Optional `create_task` fields:

- `description`
- `status`
- `base_priority`
- `dynamic_priority_delta`
- `priority_reason`
- `depends_on`
- `blocked_by`

For `update_task`, include:

- `task_id`
- only the fields that should change
- omit `description` unless the current description is blank
