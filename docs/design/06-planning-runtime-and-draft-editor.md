# Planning Runtime And Draft Editor

This file is the technical deep dive for planning runtime implementation details.

The operator-facing current contract lives in
[../supersession/current-contract.md](../supersession/current-contract.md).

## Git-Backed Authority Model

- Git-backed workspaces resolve one canonical repo authority root and persist planning authority
  under the user-level `.akra/projects/<repo-hash>/runtime/planning-authority.db` store.
- Active planning, staged drafts, official refresh claims, distributor queue claims, and runtime slot, session, and distributor projections are repo-scoped authority-store data.
- Git-backed runtime no longer writes task authority or queue projection JSON files during normal authority updates.
- Tracked planning files under `.codex-exec-loop/planning/` remain operator-authored prompts, direction detail docs, and result-output guidance only.
- Authority inspection reports store health directly from SQLite state.

## Planning Artifacts

| Path | Ownership | Role |
| --- | --- | --- |
| SQLite direction authority | DB-backed planning authority | defines directions, detail-doc mapping, and queue-idle policy |
| `.codex-exec-loop/planning/directions/<direction-id>.md` | operator-owned through staged drafts | long-form direction detail |
| `.codex-exec-loop/planning/result-output.md` | protected planning contract | result-output guidance fragment |
| `.codex-exec-loop/planning/prompts/queue-idle-review.md` | operator-owned through staged drafts | prompt used when queue-idle review is enabled |
| `.codex-exec-loop/planning/drafts/<draft>/...` | staged workspace | inactive edits awaiting validation and promotion |
| `.codex-exec-loop/planning/rejected/<turn>/...` | runtime archive | rejected planning writes preserved for inspection |

## Technical Rules

- Accepted planning still follows `draft -> validate -> promote`; direct active-state mutation is
  not the primary authoring path.
- In git-backed workspaces, accepted task authority lives in relational task tables behind the
  application `PlanningTaskRepositoryPort`.
- Manual submit and auto follow-up both append the same accepted planning prompt fragment.
- Proposed tasks do not enter the executable queue until they are promoted or otherwise moved into
  normal queue state.
- Builtin `next-task` uses the accepted queue head only.
- Queue-idle behavior is driven by DB direction authority.
- `PriorityQueueService` builds the domain `PriorityQueueProjection`; the projection owns
  queue/proposal summary facts, while prompt sections and shell copy remain outside the domain.

## Runtime Task Intake

Runtime task intake is the narrow operator path for adding one user-authored task while the shell is
already running. It is intentionally separate from broad planning authoring: `:planning` remains the
staged-draft surface, while task intake creates a single validated task mutation against the accepted
task authority.

The TUI no longer exposes task intake as an inline command. Admin/API task creation and manual
prompt intake paths reuse the same `PlanningTaskIntakeRequest` -> draft -> validation -> commit
path. A committed runtime task never interrupts an existing
`in_progress` task; it enters as a normal `ready` candidate for the next queue selection.

The intake authority flow is:

1. Admin/API or manual prompt input becomes a `PlanningTaskIntakeRequest`.
2. `PlanningTaskDraftGenerator` converts the prompt into one `PlanningTaskIntakeDraft`.
3. `PlanningTaskIntakeValidationService` validates the draft shape, selected direction, task id,
   priority, and dependency references.
4. The service appends the accepted task to the current ledger, then runs the existing
   `PlanningValidationService` and `PriorityQueueService` over the full ledger and direction catalog.
5. `PlanningTaskRepositoryPort` commits the accepted ledger and rebuilt queue projection in one
   revision-aware task-authority mutation.

LLM or hidden-session output is never allowed to write SQL or tracked planning JSON directly. It may
only implement `PlanningTaskDraftGenerator` and return a structured
`PlanningTaskIntakeDraft`; the same validation helper and accepted mutation path must handle every
generator.

The v1 generator is `LocalPromptTaskDraftGenerator`. It derives a stable title and description from
the operator prompt, sets `status=ready`, `created_by=user`, `last_updated_by=user`,
`base_priority=80`, `dynamic_priority_delta=0`, empty dependency and blocker lists, and
`source_turn_id` from the active turn when present. The default direction is `general-workstream`
when it is active; otherwise it uses the first active direction. If no active direction exists, or if
the planning workspace is missing, intake is rejected with guidance to open `:directions` or
`:planning`. Intake can pause the current internal continuation cycle, but it does not expose a user-facing automation toggle.

Task ids use `task-user-<UTC timestamp>-<prompt hash>` with a numeric suffix on collision. The
timestamp must be UTC in compact sortable `YYYYMMDDTHHMMSSZ` form, and the hash must be derived from
the normalized prompt, not from generated preview text.

The task-authority commit must be revision-aware. The intake service loads a planning revision with
the ledger and queue projection, validates against that view, and commits with compare-and-commit
semantics. If another accepted planning mutation lands first, user intake reloads the latest
snapshot, regenerates any colliding id suffix, revalidates, and retries within a bounded loop.

## Protection And Recovery Rules

- DB direction authority and `result-output.md` are protected during automated execution.
- Invalid hidden-session task authority payloads are rejected and may trigger a bounded repair retry.
- Queue refresh and repair work run through the planning worker boundary.
- If the queue is valid but idle, runtime behavior follows `queue_idle.policy`.
- If automation sees the same accepted queue head again, queue-driven follow-up pauses until the queue advances.

## Current Limits

- Non-git workspaces still use workspace-local authority storage instead of a shared repo-scoped store.
- Operator-edited planning support files require explicit draft promotion or admin apply before they are accepted.
- Real-terminal validation is still required for restart recovery, distributor delivery, and multi-worktree operator flow.
- The checked-in schema snapshot still predates newer app-server approval response methods, so the TUI does not expose approve or deny actions yet.

## Current Authority Baseline

- Repo-shared SQLite planning authority is the current implementation for git-backed workspaces.
- Historical pre-store redesign notes are no longer kept as active docs; use this file and
  [../supersession/current-contract.md](../supersession/current-contract.md) for the shipped
  authority contract.

## Code Entry

- Application entrypoint: `src/application/service/planning`
- Planning authority port: `src/application/port/outbound/planning_authority_port.rs`
- Planning task repository port: `src/application/port/outbound/planning_task_repository_port.rs`
- Planning authority adapter: `src/adapter/outbound/db/sqlite_planning_authority_adapter.rs`
- Planning domain model and queue projection: `src/domain/planning`
- TUI entrypoint: `src/adapter/inbound/tui/app/planning`
- CLI lifecycle entrypoint: `src/adapter/inbound/cli.rs`
