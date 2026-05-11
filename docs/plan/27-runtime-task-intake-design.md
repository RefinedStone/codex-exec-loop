# Runtime Task Intake Design

This document tracks the runtime task intake service that adds one user task without opening the
broader planning draft editor. The earlier TUI inline command and overlay design has been
superseded; current task creation should enter through admin/API surfaces or a future manual prompt
intake path.

## Goal

Operators need a low-friction way to enqueue a new task while work is already underway. The intake
path must work while a turn is streaming, while queue evaluation is running, and while internal
continuation is paused. It should add a normal `ready` task and leave the current `in_progress`
task alone.

The first implementation is local and deterministic. Future hidden Codex sessions or LLM structured
output adapters may help draft a task, but they must plug into the same draft and validation path.

## Intake UX

| Surface | Behavior |
| --- | --- |
| Admin/API task creation | creates a draft from a prompt and commits the validated task |
| Future manual prompt intake | may reuse the same request/draft/validation/commit path |

An admin preview should show:

- generated title
- selected direction
- status
- base priority and dynamic delta
- description excerpt

The intake surface intentionally avoids multi-task editing, JSON editing, dependency editing, and
automation toggles. Those remain planning-authoring responsibilities.

## Application Contract

Add these application-level types:

- `PlanningTaskIntakeDraft`
- `PlanningTaskIntakeProposal`
- `PlanningTaskIntakeRequest`
- `PlanningTaskIntakeCommitResult`

The request carries:

- raw operator prompt
- workspace or authority root
- optional active turn id
- optional requested direction id for future UI expansion
- observed planning revision when available

The proposal carries the draft plus preview copy and any non-fatal warnings. The commit result
returns the committed task id, committed planning revision, rebuilt queue head, and export status.

## Structured Draft Contract

`PlanningTaskDraftGenerator` is the only generation seam:

```rust
pub trait PlanningTaskDraftGenerator: Send + Sync {
    fn generate(
        &self,
        request: &PlanningTaskIntakeRequest,
    ) -> anyhow::Result<PlanningTaskIntakeDraft>;
}
```

The v1 implementation is `LocalPromptTaskDraftGenerator`. It must set:

| Field | Value |
| --- | --- |
| `status` | `ready` |
| `created_by` | `user` |
| `last_updated_by` | `user` |
| `base_priority` | `80` |
| `dynamic_priority_delta` | `0` |
| `depends_on` | `[]` |
| `blocked_by` | `[]` |
| `source_turn_id` | active turn id when present |

Direction selection is:

1. use `general-workstream` when it exists and is active
2. otherwise use the first active direction in the accepted catalog
3. reject intake when no active direction exists

The title is a compact deterministic summary of the normalized prompt. The description preserves the
operator prompt in readable form. The task id is
`task-user-<UTC timestamp>-<prompt hash>`, where `<UTC timestamp>` uses compact sortable
`YYYYMMDDTHHMMSSZ` form. Add `-2`, `-3`, and so on when a collision is found during validation or
retry.

LLM output, hidden-session output, and future planner adapters may only return this structured draft.
They must not write SQL, mutate `DB task authority`, update queue projection exports, or generate
runtime exports directly.

## Validation Layer

Add `PlanningTaskIntakeValidationService` as a single-draft validator before ledger mutation. It
rejects:

- blank prompt
- missing planning workspace
- no active direction
- unknown or inactive direction
- blank title or description
- invalid priority bounds: runtime intake accepts `base_priority` from `0` through `100`,
  `dynamic_priority_delta` from `-100` through `100`, and an effective combined priority from `0`
  through `100`
- duplicate task id
- dependency or blocker ids that do not exist
- dependency or blocker self-reference

After single-draft validation succeeds, the intake service appends the task to an in-memory ledger
copy and runs the existing `PlanningValidationService` and `PriorityQueueService` against the full
ledger and direction catalog. A task is accepted only when the full ledger validates and the queue
projection rebuilds.

## Authority And Export Flow

In git-backed workspaces, SQLite remains canonical:

1. load the current task-authority snapshot from `PlanningTaskRepositoryPort`
2. build a draft from the prompt
3. validate the draft against the accepted direction catalog and ledger
4. append the task to the ledger
5. rebuild the queue projection
6. commit ledger, queue projection, and planning revision in one store transaction
7. write runtime exports from the committed store state

Task-authority exports and the legacy-named queue projection artifact remain review surfaces only.
Runtime intake does not make exported files authoritative and does not accept out-of-band edits.

The service contract treats the accepted task ledger and queue projection as one store-backed
mutation, regardless of workspace type.

## Revision-Safe Commit

Extend `PlanningTaskRepositoryPort` with a revision-aware commit API. The minimum behavior is:

- load returns `planning_revision` with the task ledger and queue projection
- commit includes the observed revision
- commit succeeds only if the stored revision still matches the observed revision
- success writes the ledger, rebuilt queue projection, exports, and next revision atomically

On conflict, user intake reloads the latest snapshot, regenerates only collision-sensitive fields
such as task id suffix, revalidates, and retries with a bounded limit. A stale queue refresh must not
commit a ledger or projection derived from an older revision over a newer runtime task.

## Runtime Semantics

- Task intake can initialize the default planning workspace when the workspace is otherwise valid,
  but it does not enable queue automation until planning files exist.
- Task intake is rejected when planning authority cannot be initialized or repaired.
- Task intake is rejected when no active direction exists; the operator should repair planning
  direction authority before adding tasks.
- If one task is already `in_progress`, queue ranking keeps that task ahead of newly added `ready`
  work.
- Commit failure leaves the accepted ledger and queue projection unchanged.

## Failure Modes

| Failure | Expected behavior |
| --- | --- |
| Blank prompt | keep the draft uncommitted and show validation copy |
| Missing workspace | reject with repair guidance; do not commit partial authority |
| No active direction | reject with planning-authority repair guidance |
| Duplicate generated id | retry with suffix before preview or commit |
| Full-ledger validation error | reject commit and keep accepted state unchanged |
| Planning revision conflict | reload, revalidate, retry user intake |
| Export write failure after store commit | report export status and allow authority inspection to repair |

## Test Plan

- unit: local prompt generator produces deterministic title, description, and default fields
- unit: intake validation rejects blank prompt, missing direction, invalid priority, duplicate id,
  and invalid dependencies
- service: adding one user task to an empty ledger updates task ledger and queue projection together
- service: concurrent planning revision conflict retries user intake against the latest snapshot
- service: stale queue refresh cannot overwrite a newer intake mutation
- Admin/API: prompt-backed task creation reaches the shared intake service
- SQLite adapter: `planning_tasks`, `planning_queue_projection`, and runtime exports reflect the
  same accepted mutation

## Related Docs

- [../design/06-planning-runtime-and-draft-editor.md](../design/06-planning-runtime-and-draft-editor.md)
- [../supersession/current-contract.md](../supersession/current-contract.md)
- [18-repo-shared-planning-authority-store.md](18-repo-shared-planning-authority-store.md)
