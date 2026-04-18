# Planning Runtime And Draft Editor

This file records the active planning contract in the current branch workspace.

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

## Operator Entry

- `akra doctor` inspects planning health from a normal shell prompt without mutating files.
- `akra init` writes the default simple planning scaffold into the active workspace.
- `akra reset {queue|directions|all}` rewrites active planning artifacts with explicit target semantics.
- `:planning` opens planning workspace controls.
- `:doctor` reports planning health inside the shell and routes absent workspaces toward initialization.
- `:init` stages the default simple scaffold review inside the shell.
- `:reset {queue|directions|all}` runs the same reset targets from inside the shell.
- `:planning on|off` toggles plan execution without deleting the workspace.
- `:directions` opens directions maintenance.
- Simple mode stages a minimal planning workspace, explains that it starts without a next task, and can promote immediately or open the draft editor.
- Detail mode opens the embedded draft editor.
- Staged drafts stay inactive until explicit promotion.

## Current Lifecycle Terms

| Term | Meaning |
| --- | --- |
| staged draft | inactive planning edits stored under `drafts/` until validation and promotion succeed |
| active planning | accepted planning files the runtime uses for prompt assembly and queue evaluation |
| queue head | the single highest-priority executable task derived from accepted planning state |
| proposed task | a follow-up candidate that is visible but not yet executable |
| rejected planning write | an invalid planning change restored out of the active workspace and archived under `rejected/` |
| repair attempt | a bounded hidden worker retry used after invalid planning changes |

## Runtime State Contract

| State | Meaning | Operator consequence |
| --- | --- | --- |
| uninitialized | planning workspace has not been promoted for this workspace | queue-driven automation cannot proceed yet |
| invalid | active planning files fail validation or are incomplete | automation stays paused until the workspace validates again |
| ready without task | planning is valid but has no actionable queue head | runtime follows `queue_idle.policy` |
| ready with task | planning is valid and has an executable queue head | manual prompt assembly and queue-driven automation can reference the task |

## Promotion And Execution Rules

- Manual submit and auto follow-up both append the same accepted planning prompt fragment.
- `queue.snapshot.json` is derived state only and is not treated as operator-authored source.
- Proposed tasks do not enter the executable queue until they are promoted or otherwise moved into normal queue state.
- Builtin `next-task` uses the accepted queue head only.
- Queue-idle behavior is driven by `[queue_idle]` in `directions.toml`.

## Lifecycle Command Contract

| Surface | Contract |
| --- | --- |
| `akra doctor`, `:doctor` | read-only planning inspection that reports `absent`, `incomplete`, `invalid`, `ready_without_task`, or `ready_with_task`, plus queue-idle policy, queue summary, proposal summary, and the first blocking issue when relevant |
| `akra init` | creates the default simple scaffold directly in active planning files and refuses to overwrite an existing workspace |
| `:init` | stages the same default scaffold for in-shell review; when a workspace already exists it reuses planning controls instead of overwriting files |
| `akra reset queue` | rewrites `task-ledger.json` and clears derived queue state |
| `akra reset directions` | rewrites directions-side defaults, removes generated direction detail docs and queue-idle prompt artifacts, and refuses when live non-done tasks still exist |
| `akra reset all` | replaces the full active planning scaffold and clears derived queue state |
| `:reset` | uses the same reset targets; `directions` and `all` require explicit `confirm` before the shell applies them |

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
