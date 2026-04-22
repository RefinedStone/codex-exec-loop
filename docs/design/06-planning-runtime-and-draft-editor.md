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
| `.codex-exec-loop/planning/task-ledger.json` | shared operator/runtime contract | task source of truth |
| `.codex-exec-loop/planning/task-ledger.schema.json` | protected planning contract | task-ledger validation schema |
| `.codex-exec-loop/planning/result-output.md` | protected planning contract | result-output guidance fragment |
| `.codex-exec-loop/planning/prompts/queue-idle-review.md` | operator-owned through staged drafts | prompt used when queue-idle review is enabled |
| `.codex-exec-loop/planning/queue.snapshot.json` | explicit import and review surface in git-backed mode | executable queue projection artifact only |
| `.codex-exec-loop/planning/drafts/<draft>/...` | staged workspace | inactive edits awaiting validation and promotion |
| `.codex-exec-loop/planning/rejected/<turn>/...` | runtime archive | rejected planning writes preserved for inspection |
| `.codex-exec-loop/runtime/exports/planning-snapshot.json` | runtime-derived export | full store-backed planning snapshot for diagnostics and review |
| `.codex-exec-loop/runtime/exports/task-ledger.json` | runtime-derived export | convenience export for the accepted task ledger |
| `.codex-exec-loop/runtime/exports/queue.snapshot.json` | runtime-derived export | convenience export for the accepted queue projection |

## Technical Rules

- Accepted planning still follows `draft -> validate -> promote`; direct active-state mutation is
  not the primary authoring path.
- Manual submit and auto follow-up both append the same accepted planning prompt fragment.
- `queue.snapshot.json` is derived state only and is not treated as operator-authored source.
- Proposed tasks do not enter the executable queue until they are promoted or otherwise moved into
  normal queue state.
- Builtin `next-task` uses the accepted queue head only.
- Queue-idle behavior is driven by `[queue_idle]` in `directions.toml`.

## Protection And Recovery Rules

- `directions.toml`, `task-ledger.schema.json`, `result-output.md`, and `queue.snapshot.json` are protected during automated execution.
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
- Planning authority adapter: `src/adapter/outbound/sqlite_planning_authority_adapter.rs`
- TUI entrypoint: `src/adapter/inbound/tui/app/planning`
- CLI lifecycle entrypoint: `src/adapter/inbound/cli.rs`
