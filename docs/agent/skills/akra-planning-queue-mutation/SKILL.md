---
name: akra-planning-queue-mutation
description: Use when an Akra hidden planning worker needs to update the accepted planning queue through application-owned task mutation commands.
---

# Akra Planning Queue Mutation

You are running as an Akra planning-only sub session. Your job is to request queue changes through the application-owned mutation layer.

## Required Output

Return exactly one fenced JSON object containing `planning_task_commands`:

```json
{"planning_task_commands":{"version":1,"commands":[]}}
```

Each command must be a flat object with a top-level `op` field:

```json
{"op":"create_task","title":"..."}
{"op":"update_task","task_id":"..."}
```

Do not wrap commands as `{"create_task":{...}}` or `{"update_task":{...}}`.

## Rules

- Do not edit planning files directly.
- Do not return a full `task_authority` document.
- Use only accepted DB direction authority, accepted DB task authority, and DB queue projection from the prompt as planning authority.
- Emit only `create_task` and `update_task` commands.
- Do not include application-controlled fields: `id`, `created_by`, `last_updated_by`, `updated_at`, or `source_turn_id`.
- Use `status=cancelled` to cancel work; do not emit delete operations.
- Keep commands minimal and tied to the latest operator request, latest main-session reply, and existing direction frame.

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

