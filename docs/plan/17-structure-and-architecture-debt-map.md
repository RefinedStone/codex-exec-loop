# Structure And Architecture Debt Map

This document maps structural debt to operator-visible product costs so future refactors stay tied
to UX and reliability outcomes.

## Guiding Principle

Refactors are justified when they make the shell easier to reason about, easier to evolve safely, or
easier to recover when something goes wrong.

This is not a style-cleanup list. It is a product-facing debt map.

Small-context readability is part of that product cost. If a flow requires opening a giant service file
plus unrelated infrastructure adapters before the behavior becomes legible, the structure is already
hurting both implementation safety and review quality.

## Current Hotspot Order

Use this order for the current cycle when choosing a refactor slice. Completed checkpoints stay here
only when they prevent repeated work.

1. parallel-mode integration-style test follow-up only when behavior changes touch recovery, lease,
   supervisor, or dispatch behavior
2. planning runtime follow-up only when behavior changes touch validation or prompt assembly
3. parallel mode follow-up only when behavior changes touch delivery, pool cleanup, or supervisor
   wiring

If a change starts with a later hotspot, record why an earlier hotspot was skipped.

## Debt Map

| Area | Current hotspot | Operator-facing cost | Target boundary |
| --- | --- | --- | --- |
| shell presentation | `src/adapter/inbound/tui/app/shell_presentation/*` | the broad surface is now split, but overlay copy and layout still require several neighboring files | keep wording, projection, and layout modules separate |
| conversation runtime | `src/adapter/inbound/tui/app/conversation_runtime.rs`, `conversation_model.rs`, `conversation_model/view_model.rs` | current file sizes are controlled, but continuation and conversation status still need clear ownership when behavior changes | separate conversation lifecycle from continuation lifecycle and surface projection |
| planning controller | `src/adapter/inbound/tui/app/planning/controller.rs`, `controller/*`, `planning_draft_editor_ui.rs` | setup keys, status copy, and close-risk helpers are split; future work should avoid re-coupling effectful controller paths | keep setup, authoring, and close-risk handling in dedicated modules |
| parallel mode service | `src/application/service/parallel_mode/pool.rs`, `src/application/service/parallel_mode/distributor.rs`, `src/application/service/parallel_mode/session_detail.rs` | pool board projection and distributor snapshot projection are split, but delivery and cleanup flows should stay narrow when they change | keep service modules focused on orchestration and keep pure readiness, roster, detail, slot, and cleanup decisions in `src/domain/parallel_mode` |
| planning runtime services | `src/application/service/planning/authoring/directions.rs`, `src/application/service/planning/runtime/validation.rs`, `src/application/service/planning/repair/reconciliation.rs`, `src/application/service/planning/runtime/prompt.rs` | planning concepts are powerful but spread across services with overlapping product language | keep semantic validation and queue facts in `src/domain/planning`; distinguish authoring, validation, runtime prompt assembly, and recovery more sharply |
| continuation policy and queue behavior | `src/application/service/planning/runtime/policy.rs`, `src/application/service/planning/runtime/facade.rs`, `src/application/service/planning/worker/orchestration.rs` | queue-driven continuation is hard to explain because policy, prompting, and recovery are separated differently than the operator sees them | align continuation policy surface with operator concepts such as next task, pause reason, and resume path |
| outbound infrastructure layout | `src/adapter/outbound/` as one flat directory | DB, GitHub, filesystem, and app-server details are harder to skip when tracing feature logic | group outbound adapters by infrastructure boundary and keep composition near entrypoints |
| broad integration-style tests | `src/adapter/inbound/tui/app/shell_rendering*_tests.rs`, `src/application/service/parallel_mode/tests/*`, and planning runtime test clusters | behavior is well-covered, but intent is harder to discover for future changes | split tests by operator journey and subsystem contract |

## Domain Extraction Status

Recent extraction work moved several formerly service-local calculations into domain types:

- `src/domain/parallel_mode.rs` now derives supervisor state and pool slot state from lease state,
  cleanup-ready decisions, roster entries, selected detail, and live-detail enrichment; readiness
  capability models live in `src/domain/parallel_mode/readiness.rs`.
- `src/domain/planning` now owns planning semantic validation, priority queue ordering and
  skip/proposal classification, queue visibility, and queue/proposal summary projection.
- The application layer should therefore focus new parallel-mode and planning work on orchestration,
  persistence, prompt assembly, and recovery side effects.

## Completed Boundary Checkpoints

- All non-reference source files under `src/` are below the 1000-line threshold.
- `src/adapter/inbound/tui/app/planning/controller.rs` is split into overlay key handlers, draft
  editor actions, and status/reset copy helpers under `planning/controller/`.
- `src/adapter/inbound/tui/app/conversation_model/view_model.rs` keeps warning, runtime-notice,
  approval, and control support status helpers in `conversation_model/view_model/status.rs`, and
  message buffering/live-message projection helpers in `conversation_model/view_model/messages.rs`.
- `src/adapter/outbound/filesystem/planning_workspace.rs` no longer imports the concrete SQLite
  authority adapter; repo-scoped workspace behavior is injected through
  `RepoScopedPlanningWorkspacePort`.
- `src/adapter/outbound/db/sqlite_planning_authority_adapter.rs` keeps active-document,
  draft-file staging, runtime-projection, repo-scoped-workspace, store, and path helpers in child
  modules, and task-authority row persistence in
  `sqlite_planning_authority_adapter/task_authority_rows.rs`.
- `src/adapter/outbound/github/review_poller.rs` keeps parser, credential, and snapshot mapping
  tests in `github/review_poller/tests.rs`.
- `src/application/service/planning/repair/reconciliation.rs` keeps guard tests and fixtures in
  `repair/reconciliation/tests.rs`.
- `src/application/service/planning/authoring/directions.rs` keeps supporting-file path validation,
  default detail-doc generation, queue-idle prompt normalization, and catalog path mutation helpers
  in `authoring/directions/supporting_files.rs`.
- `src/application/service/planning/shared/planning_paths.rs` is the shared owner for planning
  markdown path validation used by runtime validation and directions authoring.
- `src/domain/session_browser.rs` keeps search tokenization and ranking in
  `domain/session_browser/search.rs` and browser projection tests in
  `domain/session_browser/tests.rs`, leaving state, projection, project filtering, and page
  selection in the parent domain module.
- `src/domain/parallel_mode.rs` keeps readiness capability keys, states, and snapshots in
  `domain/parallel_mode/readiness.rs` and domain contract tests in `domain/parallel_mode/tests.rs`.
- `src/application/service/planning/runtime/prompt.rs` keeps prompt fragment projection in
  `runtime/prompt/fragment.rs`.
- `src/application/service/planning/runtime/validation.rs` keeps workspace validation contract tests
  in `runtime/validation/tests.rs`.
- `src/application/service/planning/runtime/intake.rs` keeps local runtime task draft generation,
  prompt normalization, title/id derivation, and generator contract tests in
  `runtime/intake/draft.rs`.
- `src/application/service/planning/task_mutation.rs` keeps command extraction in
  `task_mutation/commands.rs`, mutation tests in `task_mutation/tests.rs`, and task mutation helper
  rules in `task_mutation/helpers.rs`.
- `src/application/service/planning/worker/orchestration.rs` keeps planning worker prompt assembly
  and prompt contract tests in `worker/orchestration/prompts.rs`.
- `src/application/service/parallel_mode/distributor.rs` keeps snapshot, orchestrator status,
  rebase provenance, and completion-feed projection in `parallel_mode/distributor/snapshot.rs`.
- `src/application/service/parallel_mode/distributor/delivery.rs` keeps integration worktree,
  patch-equivalence, and cherry-pick conflict helpers in `parallel_mode/distributor/delivery/integration.rs`;
  GitHub push, PR ensure, and merge-readiness stages live in
  `parallel_mode/distributor/delivery/github.rs`.
- `src/application/service/parallel_mode/pool.rs` keeps pool-board projection helpers in
  `parallel_mode/pool/board.rs`, slot lease mirror persistence in
  `parallel_mode/pool/lease_store.rs`, and cleanup/reset helpers in
  `parallel_mode/pool/cleanup.rs`.
- `src/application/service/parallel_mode/mod.rs` keeps agent branch naming, slug truncation, and
  branch collision checks in `parallel_mode/branch_names.rs`, dispatch/blocker helpers in
  `parallel_mode/orchestration.rs`, and shared git/fs support helpers in `parallel_mode/support.rs`.
- `src/application/service/parallel_mode/session_detail.rs` keeps session-detail file persistence,
  path sanitization, and history append deduplication in `parallel_mode/session_detail/store.rs`.
- `src/adapter/inbound/tui/app/inline_terminal_adapter/tests.rs` keeps history flush suffix/sync
  contracts in `inline_terminal_adapter/tests/history_flush.rs` and shared test app fixtures in
  `inline_terminal_adapter/tests/fixtures.rs`.
- `src/adapter/inbound/tui/app/inline_shell_commands.rs` keeps parser, palette, hint, and help
  tests in `inline_shell_commands/tests.rs`.
- `src/adapter/inbound/tui/app/shell_rendering_contract_tests.rs` keeps planning overlay/editor
  rendering contracts in `shell_rendering_contract_tests/planning.rs` and shared rendering test
  fixtures in `shell_rendering_contract_tests/fixtures.rs`.
- `src/adapter/inbound/tui/app/shell_runtime/tests.rs` keeps redraw scheduler contracts in
  `shell_runtime/tests/scheduler.rs` and input/palette key contracts in
  `shell_runtime/tests/input.rs`.
- `src/adapter/inbound/tui/app/github_polling.rs` keeps review polling state/bootstrap tests in
  `github_polling/tests.rs`.
- `src/application/service/parallel_mode/tests/distributor/mod.rs` keeps blocked/retry/patch
  equivalence distributor queue contracts in `parallel_mode/tests/distributor/blocked.rs`.
- `src/application/service/parallel_mode/tests/pool/mod.rs` keeps reconciliation/provision/reset
  and cleanup contracts in `parallel_mode/tests/pool/reconciliation.rs`.
- `src/adapter/inbound/admin_api/mod.rs` keeps JSON planning API handlers in
  `admin_api/api.rs`, and CSRF/render/redirect helpers in `admin_api/helpers.rs`, leaving server
  bootstrap, router wiring, and page handlers in the parent module.
- `src/adapter/inbound/telegram_bot/mod.rs` keeps command parsing and parser-only help fallback in
  `telegram_bot/message.rs`, and CLI/environment configuration loading in
  `telegram_bot/config.rs`.
- `src/adapter/outbound/app_server/protocol.rs` keeps active-turn app-server notification
  translation in `app_server/protocol/turn_notifications.rs`.
- `src/adapter/outbound/app_server/connection.rs` keeps pending notification buffering and
  stderr/warning diagnostics in `app_server/connection/diagnostics.rs`.
- `src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution.rs` keeps planner
  worker panel projection helpers in `turn_submission_runtime/post_turn_execution/planner_worker_panel.rs`.
- `src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution.rs` keeps hidden planning
  repair retries in `turn_submission_runtime/post_turn_execution/repair.rs`.
- `src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution.rs` keeps official
  completion capture and refresh handling in
  `turn_submission_runtime/post_turn_execution/official_completion.rs`.
- `src/adapter/inbound/tui/app/planning_draft_editor_ui.rs` keeps editor state contract tests in
  `planning_draft_editor_ui/tests.rs`.

## Planning Hotspot Audit

Composition wiring is no longer the planning hotspot. The planning feature composition now delegates
shared services plus workspace, runtime, and worker dependency bundles before constructing public
use-case groups.

The remaining planning hotspots by current implementation size and mixed responsibility are:

| Rank | Hotspot | Current pressure | Narrow next slice |
| --- | --- | --- | --- |
| 1 | parallel-mode integration-style tests | distributor and pool test clusters now have first-pass subsystem splits; remaining files are narrower recovery, lease, supervisor, and dispatch contracts | split only when a behavior change touches one of those contracts |
| 2 | `src/application/service/planning/admin/*` and `src/adapter/inbound/admin_api/*` | admin facade and adapter API/page boundaries are split; remaining pressure should come from a concrete admin behavior change, not speculative DTO churn | keep admin submodules stable and move only clearly isolated admin projections or document helpers |
| 3 | `src/application/service/planning/runtime/validation.rs`, `src/application/service/planning/runtime/prompt.rs`, and `src/application/service/planning/runtime/intake.rs` | validation, prompt assembly, and intake draft generation are now below line pressure and have shared path/prompt/draft projection owners; future pressure should come from behavior changes | extract additional rule groups only when a behavior change touches them |

Queued next narrow slice:

- **Task:** Split remaining parallel-mode integration-style tests only when a behavior change touches
  recovery, lease, supervisor, or dispatch behavior.
- **Why next:** line-limit, controller, repair, directions, runtime prompt/path, shell rendering,
  shell runtime, inline terminal history, pool board, distributor snapshot, distributor blocked
  queue, and pool reconciliation checkpoints are complete; remaining test files are narrower.
- **Target write set:** one `src/application/service/parallel_mode/tests/*` cluster plus any local
  test helpers needed to keep fixture setup readable.
- **Acceptance:** the same behavior remains covered, but a future change can find the relevant test
  without opening unrelated journeys.

## Chosen Boundary Model

### 1. Shell Runtime Boundary

Owns:

- startup readiness
- active turn lifecycle
- session selection and shell mode transitions

Should not own:

- planning authoring semantics
- detailed continuation copy decisions

### 2. Continuation Boundary

Owns:

- stop rules
- queue-driven continuation decision
- pause reason projection
- preview assembly contract

Should not own:

- planning authoring flow
- generic shell layout

### 3. Planning Authoring Boundary

Owns:

- staging
- editing
- promoting
- close-risk and dirty-buffer semantics
- direction and queue-idle supporting files

Should not own:

- post-turn runtime evaluation
- generic queue explanation

### 4. Planning Runtime Boundary

Owns:

- accepted planning snapshot
- queue derivation
- proposed-task semantics
- runtime readiness states

Should not own:

- shell copy scattered across presentation layers

### 5. Repair And Worker Boundary

Owns:

- reconciliation
- rollback
- bounded hidden worker retries
- rejected artifact preservation

Should expose:

- why a change was rejected
- what was restored
- what action the operator should take next

## Sequencing Recommendation

### Stream A: Product Language First

Before deep refactors, normalize operator vocabulary in docs and presentation surfaces so later code moves have a stable target.
Use shared terms such as direction, queue task, proposed task, accepted planning, queue-idle policy, and repair before moving files around.

### Stream B: Presentation And Status Extraction

Split shell wording and projection concerns from layout and rendering concerns. This makes UX work cheaper without changing planning internals first.

### Stream C: Infrastructure Directory Separation

Move outbound adapters into clear technology boundaries so feature-level analysis can skip infra details unless a task really crosses that boundary.

### Stream D: Parallel Mode And Planning Service Split

Separate parallel-mode responsibilities and planning authoring/runtime/repair concerns so both flows become smaller and more legible to change.

### Stream E: Runtime Safety And Test Shape

Reorganize runtime tests around operator journeys and architectural contracts rather than only around current file boundaries.

## Definition Of Done For Each Refactor Slice

- one clear ownership boundary becomes easier to explain in docs
- one operator-visible flow becomes easier to reason about
- one hotspot file or cluster loses mixed responsibilities
- one infrastructure directory becomes easier to skip when it is not relevant to the current feature
- matching tests describe user-visible intent more clearly than before

## Explicit Assumptions

- The inline shell remains the only frontend in the near term.
- Planning remains operator-owned, but the long-term trusted source of continuity may move to a
  repo-shared authority store as long as staged drafts and exported review surfaces remain intact.
- Architectural work is only worth taking when it improves UX clarity, planning trust, or runtime safety.
