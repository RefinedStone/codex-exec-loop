# Planning Runtime And Draft Editor

This file is the technical deep dive for planning runtime implementation details.

The operator-facing current contract lives in
[../supersession/current-contract.md](../supersession/current-contract.md).

## Git-Backed Authority Model

- Git-backed workspaces resolve one canonical repo authority root and persist planning authority under `.codex-exec-loop/runtime/planning-authority.db`.
- Active planning, staged drafts, official refresh claims, distributor queue claims, and runtime slot, session, and distributor projections are repo-scoped authority-store data.
- Git-backed runtime writes exported review views under `.codex-exec-loop/runtime/exports/` and no longer rewrites tracked planning files during normal authority updates.
- Tracked planning files under `.codex-exec-loop/planning/` remain explicit import, review, and portability artifacts for git-backed workspaces, while non-git workspaces still use direct local planning files.
- Authority inspection can repair runtime export views from store truth when they drift or disappear.

## Planning Artifacts

| Path | Ownership | Role |
| --- | --- | --- |
| `.codex-exec-loop/planning/directions.toml` | operator-owned through staged drafts | defines directions, detail-doc mapping, and queue-idle policy |
| `.codex-exec-loop/planning/directions/<direction-id>.md` | operator-owned through staged drafts | long-form direction detail |
| `.codex-exec-loop/planning/task-ledger.json` | explicit import/review surface in git-backed mode | task ledger interchange artifact |
| `.codex-exec-loop/planning/task-ledger.schema.json` | protected planning contract | task-ledger validation schema |
| `.codex-exec-loop/planning/result-output.md` | protected planning contract | result-output guidance fragment |
| `.codex-exec-loop/planning/prompts/queue-idle-review.md` | operator-owned through staged drafts | prompt used when queue-idle review is enabled |
| `.codex-exec-loop/planning/queue.snapshot.json` | legacy-named explicit import and review surface in git-backed mode | queue projection artifact only |
| `.codex-exec-loop/planning/drafts/<draft>/...` | staged workspace | inactive edits awaiting validation and promotion |
| `.codex-exec-loop/planning/rejected/<turn>/...` | runtime archive | rejected planning writes preserved for inspection |
| `.codex-exec-loop/runtime/exports/planning-snapshot.json` | runtime-derived export | full store-backed planning snapshot for diagnostics and review |
| `.codex-exec-loop/runtime/exports/task-ledger.json` | runtime-derived export | convenience export for the accepted task ledger |
| `.codex-exec-loop/runtime/exports/queue.snapshot.json` | legacy-named runtime-derived export | convenience export for the accepted queue projection |

## Technical Rules

- Accepted planning still follows `draft -> validate -> promote`; direct active-state mutation is
  not the primary authoring path.
- In git-backed workspaces, accepted task authority lives in relational task tables behind the
  application `PlanningTaskRepositoryPort`; tracked `task-ledger.json` is accepted only through an
  explicit import or promoted draft.
- Manual submit and auto follow-up both append the same accepted planning prompt fragment.
- Queue projection exports are derived state only. The legacy `queue.snapshot.json` filename is a
  compatibility artifact, not an operator-authored planning concept.
- Proposed tasks do not enter the executable queue until they are promoted or otherwise moved into
  normal queue state.
- Builtin `next-task` uses the accepted queue head only.
- Queue-idle behavior is driven by `[queue_idle]` in `directions.toml`.

## Runtime Task Intake

Runtime task intake is the narrow operator path for adding one user-authored task while the shell is
already running. It is intentionally separate from broad planning authoring: `:planning` remains the
staged-draft surface, while `:task` creates a single validated task mutation against the accepted
task authority.

The TUI exposes the intake as `:task`. `:task <prompt>` opens a preview for that prompt immediately;
plain `:task` opens an intake overlay with a prompt editor. The overlay shows title, direction,
status, priority, and a description excerpt, then accepts only `Y` to commit, `N` or `Esc` to cancel,
and `E` to return to prompt editing. The command remains available during a streaming turn, queue
evaluation, and automation-stopped state. A committed runtime task never interrupts an existing
`in_progress` task; it enters as a normal `ready` candidate for the next queue selection.

The intake authority flow is:

1. TUI prompt input becomes a `PlanningTaskIntakeRequest`.
2. `PlanningTaskDraftGenerator` converts the prompt into one `PlanningTaskIntakeDraft`.
3. `PlanningTaskIntakeValidationService` validates the draft shape, selected direction, task id,
   priority, and dependency references.
4. The service appends the accepted task to the current ledger, then runs the existing
   `PlanningValidationService` and `PriorityQueueService` over the full ledger and direction catalog.
5. `PlanningTaskRepositoryPort` commits the accepted ledger and rebuilt queue projection in one
   revision-aware task-authority mutation.
6. Git-backed workspaces export `.codex-exec-loop/runtime/exports/task-ledger.json` and
   the legacy-named queue projection export from the committed store revision.

LLM or hidden-session output is never allowed to write SQL, tracked JSON, or runtime exports
directly. It may only implement `PlanningTaskDraftGenerator` and return a structured
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
snapshot, regenerates any colliding id suffix, revalidates, and retries within a bounded loop. Queue
refresh or export work that was computed from a stale revision must not overwrite a newer intake
task.

## Protection And Recovery Rules

- `directions.toml`, `task-ledger.schema.json`, `result-output.md`, and queue projection exports
  are protected during automated execution.
- Invalid `task-ledger.json` writes are rolled back, archived, and may trigger a bounded repair retry.
- Queue refresh and repair work run through the planning worker boundary.
- If the queue is valid but idle, runtime behavior follows `queue_idle.policy`.
- If automation sees the same accepted queue head again, queue-driven follow-up pauses until the queue advances.

## Current Limits

- Non-git workspaces still fall back to direct local planning files instead of the repo-scoped authority store.
- Runtime export views can still drift when edited out of band and may require authority inspection to restore parity; tracked planning files require explicit import if the operator wants them accepted again.
- Real-terminal validation is still required for restart recovery, distributor delivery, and multi-worktree operator flow.
- The checked-in schema snapshot still predates newer app-server approval response methods, so the TUI does not expose approve or deny actions yet.

## Historical Redesign References

- The repo-shared authority migration described in [../plan/18-repo-shared-planning-authority-store.md](../plan/18-repo-shared-planning-authority-store.md) is now implemented on this branch.
- The pre-cutover failure record in [../plan/19-supersession-runtime-risk-audit.md](../plan/19-supersession-runtime-risk-audit.md) should be read as historical context for the issues the authority-store cutover addressed.

## Code Entry

- Application entrypoint: `src/application/service/planning`
- Planning authority port: `src/application/port/outbound/planning_authority_port.rs`
- Planning task repository port: `src/application/port/outbound/planning_task_repository_port.rs`
- Planning authority adapter: `src/adapter/outbound/db/sqlite_planning_authority_adapter.rs`
- TUI entrypoint: `src/adapter/inbound/tui/app/planning`
- CLI lifecycle entrypoint: `src/adapter/inbound/cli.rs`
