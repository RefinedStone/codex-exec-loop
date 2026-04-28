# Current Supersession And Planning Contract

This file is the canonical current contract for the operator-facing supersession, planning, and
directions behavior on the current branch.

## Snapshot

- `origin/prerelease` already ships the first operator-facing supersession loop.
- The current branch adds repo-scoped planning authority follow-through on top of that loop.
- One inline shell now carries startup readiness, session resume, accepted planning, queue state,
  internal post-turn continuation, and supersession supervision together.

## Operator Surfaces

| Surface | Entry | Current contract |
| --- | --- | --- |
| Diagnostics | `Ctrl+d`, `:diag` | shows startup readiness and blocking failures |
| Sessions | `Ctrl+o`, `:sessions` | resumes prior threads with current-work context |
| Queue | `:queue` | shows the current queue task, proposed tasks, and skip framing |
| Task Intake | `:task` | adds one validated user task to the accepted queue without opening draft authoring |
| Planning | `:planning` | stages or reopens planning authoring flows |
| Directions | `:directions` | maintains direction-side artifacts and queue-idle supporting files |
| Parallel | `:parallel on` | opens the supersession readiness gate, supervisor board, and worker pool flow |

## Planning And Directions

Accepted planning stays on the `draft -> validate -> promote` contract.

| Command | Current contract |
| --- | --- |
| `:doctor`, `akra doctor` | read-only planning inspection |
| `:init`, `akra init` | create or stage the default planning scaffold |
| `:reset queue` | rewrites the accepted task ledger and clears derived queue state |
| `:reset directions` | rewrites direction-side defaults and removes generated supporting artifacts |
| `:reset all` | replaces the full planning scaffold and clears derived queue state |
| `:planning on|off` | toggles plan execution without deleting the workspace |
| `:task [prompt]` | opens runtime task intake, previews one user task, then commits it as accepted `ready` work |

Key artifacts:

| Path | Meaning |
| --- | --- |
| SQLite direction authority | directions, detail-doc mapping, and queue-idle policy |
| `.codex-exec-loop/planning/directions/<direction-id>.md` | long-form direction detail |
| `.codex-exec-loop/planning/prompts/queue-idle-review.md` | queue-idle supporting prompt |
| `.codex-exec-loop/planning/drafts/<draft>/...` | staged inactive edits awaiting promote |
| `.codex-exec-loop/planning/rejected/<turn>/...` | archived invalid planning writes |

Queue behavior:

- Builtin `next-task` and internal post-turn continuation use the accepted queue head only.
- Proposed tasks stay visible but are not executable queue items yet.
- Runtime task intake creates accepted `ready` tasks through a structured draft, validation, and
  revision-safe commit path; LLM output may only produce `TaskDraft` data and must not write SQL or
  JSON authority surfaces directly.
- When the queue is valid but idle, behavior follows DB direction authority queue-idle policy.
- `:directions` keeps direction detail docs and queue-idle prompt editing subordinate to the same
  staged-draft authoring model.

## Supersession Runtime

- `:parallel on` checks git, worktree, `akra`, push, `gh`, and planning readiness before entry.
- The supervisor board shows capability, pool, roster, selected-detail, and distributor projections.
- Queue-driven work leases one of three `akra` worktree slots.
- Agent work reaches `commit ready`, then hidden planning refresh decides official completion.
- Distributor delivery remains serial: rebase, push, PR automation, merge integration, and slot
  cleanup happen in one ordered lane.

For git-backed workspaces:

- repo-scoped planning authority lives under `.codex-exec-loop/runtime/planning-authority.db`
- tracked files under `.codex-exec-loop/planning/` are review, portability, export, and supported
  staged-edit artifacts rather than runtime authority
- authority inspection can repair exported review files from store truth when they drift

## Current Limits

- Non-git workspaces still use workspace-local authority storage instead of the repo-scoped store.
- Real-terminal validation is still required for restart recovery, distributor delivery, and
  multi-worktree operator flow.
- Planning detail mode remains manual authoring only; the `llm-assisted` path is still disabled.
- The checked-in schema snapshot still predates newer app-server approval response methods, so the
  TUI does not expose approve or deny actions yet.

## Deep Dives

- [../design/01-current-product-state.md](../design/01-current-product-state.md)
- [../design/02-tui-shell-flow.md](../design/02-tui-shell-flow.md)
- [../design/06-planning-runtime-and-draft-editor.md](../design/06-planning-runtime-and-draft-editor.md)
- [remaining-work.md](remaining-work.md)
