# Runtime Task Intake Design

This document is the implementation target for adding user task intake to the running TUI without
opening the broader planning draft editor.

## Goal

Operators need a low-friction way to enqueue a new task while work is already underway. The command
must work while a turn is streaming, while queue evaluation is running, and while automation is
stopped. It should add a normal `ready` task and leave the current `in_progress` task alone.

The first implementation is local and deterministic. Future hidden Codex sessions or LLM structured
output adapters may help draft a task, but they must plug into the same draft and validation path.

## Command UX

| Command | Behavior |
| --- | --- |
| `:task` | opens the Task Intake overlay prompt editor |
| `:task <prompt>` | creates a draft from `<prompt>` and opens preview immediately |

The preview overlay shows:

- generated title
- selected direction
- status
- base priority and dynamic delta
- description excerpt

Preview keys:

| Key | Behavior |
| --- | --- |
| `Y` | commit the task |
| `N` | cancel |
| `E` | return to prompt editing |
| `Esc` | cancel |

The overlay intentionally avoids multi-task editing, JSON editing, dependency editing, and
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
`task-user-<UTC timestamp>-<prompt hash>`, with `-2`, `-3`, and so on when a collision is found
during validation or retry.

LLM output, hidden-session output, and future planner adapters may only return this structured draft.
They must not write SQL, mutate `task-ledger.json`, update `queue.snapshot.json`, or generate runtime
exports directly.

## Validation Layer

Add `PlanningTaskIntakeValidationService` as a single-draft validator before ledger mutation. It
rejects:

- blank prompt
- missing planning workspace
- no active direction
- unknown or inactive direction
- blank title or description
- invalid priority bounds
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

Tracked `.codex-exec-loop/planning/task-ledger.json` and
`.codex-exec-loop/planning/queue.snapshot.json` stay import and review surfaces. Runtime intake does
not make them authoritative and does not accept out-of-band edits without the existing explicit apply
flow.

Non-git workspaces may keep their current direct-file authority path, but the service contract should
still treat ledger plus queue projection as one accepted mutation.

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

- `:task` is allowed while planning mode is off, but it does not enable automation.
- `:task` is rejected when no planning workspace exists; the TUI should guide the operator to
  `:planning`.
- `:task` is rejected when no active direction exists; the TUI should guide the operator to
  `:directions` or `:planning`.
- If one task is already `in_progress`, queue ranking keeps that task ahead of newly added `ready`
  work.
- Buffered shell commands must execute after streaming turn handling reaches a command-safe point.
- Commit failure leaves the accepted ledger and queue projection unchanged.

## Failure Modes

| Failure | Expected behavior |
| --- | --- |
| Blank prompt | keep editor open and show validation copy |
| Missing workspace | reject with `:planning` guidance; do not bootstrap implicitly |
| No active direction | reject with `:directions` or `:planning` guidance |
| Duplicate generated id | retry with suffix before preview or commit |
| Full-ledger validation error | reject commit and keep accepted state unchanged |
| Planning revision conflict | reload, revalidate, retry user intake |
| Export write failure after store commit | report export status and allow authority inspection to repair |

## Test Plan

- unit: local prompt generator produces deterministic title, description, and default fields
- unit: intake validation rejects blank prompt, missing direction, invalid priority, duplicate id,
  and invalid dependencies
- service: adding one user task to an empty ledger updates task ledger and queue snapshot together
- service: concurrent planning revision conflict retries user intake against the latest snapshot
- service: stale queue refresh cannot overwrite a newer intake mutation
- TUI: `:task`, `:task <prompt>`, preview `Y`, `N`, `E`, and `Esc`
- TUI: streaming turn buffers `:task` and executes it at the command-safe point
- SQLite adapter: `planning_tasks`, `planning_queue_projection`, and runtime exports reflect the
  same accepted mutation

## Related Docs

- [../design/06-planning-runtime-and-draft-editor.md](../design/06-planning-runtime-and-draft-editor.md)
- [../supersession/current-contract.md](../supersession/current-contract.md)
- [18-repo-shared-planning-authority-store.md](18-repo-shared-planning-authority-store.md)
