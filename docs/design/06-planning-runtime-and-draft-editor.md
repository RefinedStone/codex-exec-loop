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
- Git-backed repository identity is an owner-private 256-bit incarnation marker stored in the
  canonical Git common directory, not the checkout basename or remote URL. Linked worktrees and a
  moved common directory therefore retain one authority database, while independent repositories,
  fresh clones, bare repositories, and separate-git-dir repositories receive isolated namespaces
  even when a checkout path is reused. Marker installation is atomic/no-clobber and unsafe marker
  file types, ownership, permissions, or link counts fail closed.
- A non-Git workspace uses its canonical workspace path as the identity and still receives the same
  private SQLite authority-store protections. Windows supports both Git-backed and non-Git SQLite
  authority; only direct planning-artifact filesystem access, candidate inspection, and external
  file export/apply remain fail-closed until they use pinned NT relative handles.
- `AKRA_HOME`, when set, must be a non-empty absolute normalized path. Akra never falls back to the
  current working directory, so authority data cannot silently land inside a repository when profile
  discovery is unavailable. On Unix the selected root and its resolved ancestor chain must also be
  trusted and protected from cross-user replacement (non-writable by group/other, or sticky); Akra
  rejects an unsafe root without changing its permissions or creating managed data below it.
- The authority database and any pre-existing SQLite `-journal`, `-wal`, or `-shm` sidecar must be a
  current-user-owned, private, regular single-link file. Akra validates them before SQLite opens the
  store, checkpoints a legacy WAL store into `DELETE` rollback-journal mode, and validates the file
  family again after schema work. `DELETE` mode trades WAL reader/writer overlap for a smaller,
  auditable transient-file surface; the five-second busy timeout absorbs ordinary short planning,
  admin, and Telegram write overlap, while a persistently busy migration fails closed.

These checks prevent a pre-existing hardlink or symlink from redirecting SQLite writes. They do not
claim isolation from a malicious process already running as the same OS user: such a process can race
an absent sidecar name after preflight (and can generally access that user's data directly). OS-user
separation or an external sandbox remains the boundary for that threat.

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
- Protected-file reconciliation reads the candidate after a turn and restores its captured snapshot
  only through a storage-atomic compare-and-swap. A concurrent operator update wins, is left intact,
  and blocks continuation for explicit review instead of being overwritten by a late worker.
- Invalid hidden-session task authority payloads are rejected and may trigger a bounded repair retry.
- Queue refresh and repair work run through an ephemeral planning worker that is always
  `read-only`; validated final `planning_task_commands` are applied by the host service. A captured
  post-turn permit is watched during the hidden turn and invalidation interrupts only that worker's
  app-server process, without stopping the main or parallel sessions.
- If the queue is valid but idle, runtime behavior follows `queue_idle.policy`.
- If automation sees the same accepted queue head again, queue-driven follow-up pauses until the queue advances.

## App-Server Execution And Diagnostic Retention

App-server startup does not execute a mutable command name after discovery. Akra pins a trusted
native Codex executable, or a narrowly recognized npm launcher plus its native Node interpreter,
before creating the connection. Unix launchers must have a bounded supported Node shebang; Windows
launchers must be the bounded standard npm `.cmd` form targeting
`node_modules/@openai/codex/bin/codex.js`. Relative `PATH` entries, repository/pool-controlled
executables, unsafe owner/mode/ACL chains, arbitrary shell or batch launchers, and unsupported
interpreters fail closed. Hidden planning workers inherit the same pinned command plan in addition
to their fixed read-only sandbox and unattended-decline policy.

Native plans are not admitted by a magic prefix alone. Akra inspects the supported ELF64, Mach-O 64
(including bounded fat slices), or PE32+ tables through the already validated file handle, requires
the current release architecture, bounds metadata inspection to 16 MiB, and verifies that the image
entry belongs to an executable segment or section. The handle and path identity are rechecked after
parsing before the validated command plan is returned.

Prompt/response body persistence is a diagnostic opt-in, not a planning authority prerequisite.
When `AKRA_APP_SERVER_PROMPT_LOG=1` is absent or invalid, every production composition startup
invokes an all-row and metadata clear through the private authority SQLite connection with
`secure_delete=ON`, including unexpired rows from an earlier opted-in process. A cleanup failure
emits a body-free warning and does not enable capture. Only an enabled process retains records: at
most 100 interactions, no more than seven days, and bounded item/body content. Trace JSONL is
separately opt-in and body-redacted. These filesystem and database controls reduce
accidental/cross-user exposure but do not isolate data from a malicious process already running as
the same OS user.

Only the interactive main conversation can answer a reviewable command/additional-permission
approval. Explicit `Y` is the sole accept input; `Enter` is inert and `N`/`Esc` declines. Receipt
deadline, timeout, interrupt, disconnect, bounded-channel saturation, malformed payload, or an
unattended worker all decline, and an apparent accept rechecks deadline and interrupt immediately
before sending the response. No session-wide grant is cached.

## Current Limits

- Non-git workspaces still use workspace-local authority storage instead of a shared repo-scoped store.
- Operator-edited planning support files require explicit draft promotion or admin apply before they are accepted.
- Real-terminal validation is still required for restart recovery, distributor delivery, and multi-worktree operator flow.
- Hidden planning workers are unattended, fixed to read-only sandboxing, and decline any unexpected
  app-server approval request; only the main interactive conversation exposes the one-shot approval
  modal.

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
