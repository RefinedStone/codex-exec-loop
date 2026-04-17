# Architecture Boundaries

This document defines the target supersession model, not shipped behavior.

## Architectural Direction

Supersession should be added as a new orchestration boundary, not by letting the current single
conversation runtime absorb multi-agent, git-pool, and distributor responsibilities directly.

The stable dependency direction remains:

`adapter -> application -> domain`

## New Domain Snapshots

| Type | Role |
| --- | --- |
| `SupervisorSnapshot` | operator-facing summary of the whole supersession state |
| `CapabilitySnapshot` | git, push, GitHub, planning, and pool readiness |
| `PoolSnapshot` | slot inventory and aggregate counts |
| `SlotSnapshot` | one slot's lease, branch, worktree, and health state |
| `AgentSessionSnapshot` | one agent's lifecycle, timing, task, and latest report |
| `MergeQueueSnapshot` | current queue order and queue-head state |
| `CompletionReportSnapshot` | non-official agent report awaiting or reflecting ledger refresh |

These types should stay UI-neutral and transport-neutral.

## New Application Ports

| Port | Responsibility |
| --- | --- |
| `AgentSessionPort` | start, observe, cancel, and close main-grade agent sessions |
| `GitWorkspacePort` | inspect repo state, manage branches, and manage worktrees |
| `GithubAutomationPort` | inspect `gh` capability and run PR-oriented GitHub actions |
| `DistributorPort` | process merge queue items and clean slots |
| `PlanningAuthorityBackend` | own repo-shared planning authority, transactional queue projection, and compatibility import/export flows |
| `SupersessionRuntimeStatePort` | persist queue claims, slot and session projections, distributor delivery state, and runtime-domain events |

Planning should not remain a workspace-local file authority in supersession. Instead, a dedicated
planning authority boundary should own repo-scoped planning state and expose explicit mutation,
refresh, and export operations. The planning facade may still be the caller-facing orchestration
surface, but it must no longer assume that the active worktree owns the authoritative planning
files.

## New Application Services

- `SupersessionCapabilityService`
- `SupersessionPoolService`
- `SupersessionAssignmentService`
- `SupersessionAgentOrchestrationService`
- `SupersessionDistributorService`
- `SupersessionPresentationService`

Each service should own one orchestration concern rather than becoming a general manager.

## Reuse Boundaries

### Keep Reusing

- planning validation and runtime snapshot loading
- hidden planning worker execution port
- existing app-server transport primitives
- current TUI overlay infrastructure and shell presentation patterns

### Do Not Reuse Directly

- single `ConversationViewModel` as the source of truth for parallel mode
- hidden planning worker as the execution engine for implementation tasks
- current recent-sessions overlay state as if it were already a supervisor board

## Runtime Split

Normal mode and parallel mode should have separate runtime state reducers.

- normal mode continues to own one conversation-first runtime
- parallel mode owns supervisor, agents, pool, and merge queue state
- switching modes is a shell-level routing concern rather than a shared mutable runtime blob

## Adapter Boundaries

| Adapter | Expected implementation |
| --- | --- |
| app-server agent adapter | launches and streams agent sessions |
| git subprocess adapter | shells out to `git` for repo, branch, and worktree operations |
| planning authority adapter | resolves canonical repo scope, executes transactional planning reads and writes, and manages compatibility import/export |
| `gh` subprocess adapter | shells out to `gh` for auth, PR, and merge operations |
| TUI supervisor adapter | maps supervisor snapshots to overlay and compact shell presentation |

## Runtime Event Boundary

Supersession runtime coordination should move from ad hoc file updates toward a repo-shared event
and projection model.

- append runtime-domain events for completion, refresh, queue claim, push, PR ensure, integrate,
  cleanup, and redistribution
- project those events into slot, session, and distributor state tables
- make restart recovery consume the same projections instead of rebuilding from scattered files

This event model is for runtime orchestration only. It is not a requirement to turn all planning
authoring into full event sourcing.

## Existing Hotspots To Avoid Growing Further

- `src/adapter/inbound/tui/app/app_runtime.rs`
- `src/adapter/inbound/tui/app/shell_presentation.rs`
- `src/adapter/inbound/tui/app/conversation_runtime.rs`
- `src/application/service/session_service.rs`
- `src/adapter/outbound/codex_app_server_adapter.rs`

Supersession should reduce pressure on these hotspots by moving logic into new submodules.

## Related Docs

- [01-product-model.md](01-product-model.md)
- [04-task-ledger-feedback-loop.md](04-task-ledger-feedback-loop.md)
- [10-implementation-slices.md](10-implementation-slices.md)
- [../plan/18-repo-shared-planning-authority-store.md](../plan/18-repo-shared-planning-authority-store.md)
- [../plan/19-supersession-runtime-risk-audit.md](../plan/19-supersession-runtime-risk-audit.md)
- [../design/04-hexagonal-runtime-architecture.md](../design/04-hexagonal-runtime-architecture.md)

## Code Impact

Expected entrypoints:

- `src/application/service`
- `src/application/port/outbound`
- `src/adapter/outbound`
- `src/adapter/inbound/tui/app`
