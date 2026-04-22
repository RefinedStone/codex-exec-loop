# Supersession Runtime Risk Audit

Historical reference: use [../supersession/current-contract.md](../supersession/current-contract.md)
for the current supersession and planning contract. This file is pre-store failure analysis only.

This document recorded the main structural risks in the pre-store-primary supersession and
planning runtime.

It should now be read as historical failure analysis rather than the current target design.

## Status Note

- This audit captured the pre-store-primary supersession risk set.
- The current branch addresses R1 through R8 through repo-scoped planning authority, store-backed official refresh and distributor claims, runtime projections, and recovery rechecks.
- Keep the remaining sections as historical failure analysis for the issues the authority-store cutover closed.

## Historical Operating Envelope

The current shipped loop should be treated as effectively safe only for:

- one operator
- one app process
- one active planning authority view

It is not yet a strong multi-process or multi-worktree authority model even though some runtime
state is already repo-shared.

## Risk Matrix

| ID | Area | Failure mode | Severity | Current guardrail | Remaining gap | Target response |
| --- | --- | --- | --- | --- | --- | --- |
| R1 | planning authority | root checkout and leased worktree can observe or mutate different planning state | critical | repo-scoped refresh ordering key inside one process | planning files still resolve from active workspace path | one repo-scoped authority domain with store-backed drafts |
| R2 | planning transactionality | `task-ledger` and queue projection can diverge on disk | critical | runtime rebuilds queue in memory from accepted ledger | on-disk views still update through separate file writes | transactional promote and active-commit model |
| R3 | official completion ordering | two app processes can refresh the same repo scope independently | high | in-memory mutex and condvar gate | no cross-process coordination | claim semantics inside the same authority domain as planning revision |
| R4 | distributor serialization | duplicate queue enqueue or queue-head processing across processes | high | single-process happy-path flow | no queue-head claim or CAS | queue claims inside the same authority domain as runtime projections |
| R5 | slot and session runtime state | file-backed lease and session detail can lose concurrent updates | high | temp-write plus rename avoids partial files | lost-update race still exists | versioned runtime projections inside the authority store |
| R6 | git tracking | planning files can leak into agent branches when mutated from leased worktrees | high | protected-file restore after automated turns | authority still begins from tracked files | tracked files become export and explicit import artifacts only |
| R7 | restart recovery | in-flight refresh or delivery state can be forgotten on restart | medium | some durable pool files and session detail | ordering and claims are not restart-safe | recovery sweep plus external truth reconciliation |
| R8 | queue view drift | `queue.snapshot.json` can lag or disagree with runtime-derived queue | medium | runtime recalculates queue from ledger | humans and tools can still read stale exported files | revision-stamped exports generated from committed store state |

## R1. Worktree-Local Planning Authority

**Problem**

Planning files are loaded from the active workspace path, not from a canonical repo-scoped
authority root.

**Why it happens**

- `PlanningWorkspacePort` resolves planning files by joining `workspace_dir` with
  `.codex-exec-loop/planning/*`
- parallel handoff switches the active turn workspace into a leased worktree
- hidden planner refresh and official completion refresh continue to use that workspace path
- refresh ordering is keyed by canonical repo scope, but the actual file mutation still happens in
  the active worktree

**Failure scenario**

- root checkout shows one official queue
- a leased worktree runs hidden planner refresh
- the leased worktree mutates its own planning files or fails because they are missing
- the root checkout keeps a stale ledger or queue

**Current guardrails**

- repo-scoped refresh ordering exists inside one process
- protected-file reconciliation prevents some invalid writes

**Remaining gap**

- planning authority is still not repo-shared
- git-tracked planning files can be touched from execution worktrees

**Target response**

- move all planning authority into one repo-shared store under the canonical repo root

**Code references**

- `src/adapter/outbound/filesystem_planning_workspace_adapter.rs`
- `src/application/service/planning_prompt_service.rs`
- `src/adapter/inbound/tui/app/turn_submission_runtime/stream_execution.rs`
- `src/application/service/parallel_mode_turn_service.rs`
- `src/adapter/inbound/tui/app/turn_submission_runtime/post_turn_execution.rs`
- `src/application/service/planning_worker_orchestration_service.rs`

## R2. Non-Transactional Planning Writes

**Problem**

`task-ledger.json` and `queue.snapshot.json` are accepted, restored, and rebuilt through separate
file operations.

**Why it happens**

- workspace writes use plain filesystem replace semantics
- reconciliation restores protected files, validates the ledger candidate, and rebuilds or restores
  queue projection as later steps

**Failure scenario**

- hidden planner refresh updates `task-ledger.json`
- process crashes before `queue.snapshot.json` is rebuilt
- runtime can recompute in memory later, but exported queue files and human inspection are stale

**Current guardrails**

- runtime can rebuild queue from accepted ledger in memory
- invalid ledger writes are rolled back

**Remaining gap**

- no single atomic commit covers ledger plus projection on disk
- concurrent manual and automated writes can silently overwrite each other

**Target response**

- commit planning state and queue projection in one authority-store transaction

**Code references**

- `src/adapter/outbound/filesystem_planning_workspace_adapter.rs`
- `src/application/service/planning_reconciliation_service.rs`
- `src/application/service/planning_prompt_service.rs`

## R3. Process-Local Official Completion Ordering

**Problem**

Official completion refresh ordering is enforced only inside one app process.

**Why it happens**

- ordering uses in-memory mutex and condvar state
- refresh scope is repo-aware, but the gate is not persisted

**Failure scenario**

- two app processes open the same repo family
- both reserve and execute official completion refresh on the same logical queue
- completion ordering becomes non-deterministic

**Current guardrails**

- single-process ordering is correct
- panic-safe permit release prevents a dead in-memory gate

**Remaining gap**

- no cross-process reservation
- restart wipes ordering state

**Target response**

- replace process-local ordering with store-backed claim and revision logic

**Code references**

- `src/application/service/planning_worker_orchestration_service.rs`
- `src/application/service/parallel_mode_service.rs`

## R4. Distributor Queue Claim Gap

**Problem**

The distributor queue is file-backed but does not have a durable queue-head claim protocol.

**Why it happens**

- enqueue checks for an existing session key and then writes a new queue record
- queue processing scans records and chooses the first active item
- there is no compare-and-set claim step

**Failure scenario**

- two processes enqueue the same commit-ready result
- or two processes both start processing the same queue head
- PR ensure may remain mostly idempotent, but integrate and cleanup are not safely deduplicated

**Current guardrails**

- single-process happy path usually serializes calls
- blocked queue state stops later items in one runtime

**Remaining gap**

- no durable queue-head claim
- duplicate enqueue remains possible under race

**Target response**

- move queue claim and queue item state into a transactional authority store

**Code references**

- `src/application/service/parallel_mode_distributor_service.rs`

## R5. Lease And Session Detail Lost Updates

**Problem**

Slot leases, agent session detail, and runtime history are persisted as shared files without
optimistic versioning.

**Why it happens**

- temp-file plus rename protects against partial writes
- but read-modify-write cycles remain unclaimed

**Failure scenario**

- one worker updates session detail to `ledger_refreshing`
- another path writes `commit_ready` from an older read snapshot
- history or state transitions can be overwritten or flattened

**Current guardrails**

- invalid JSON is ignored or marked invalid
- atomic rename reduces torn-file risk

**Remaining gap**

- lost updates are still possible
- file integrity is better than state integrity

**Target response**

- use transactional projections with revision or version checks

**Code references**

- `src/application/service/parallel_mode_service.rs`
- `src/application/service/parallel_mode_distributor_service.rs`

## R6. Git-Tracked Planning Leakage

**Problem**

Planning authority files remain normal repo files and can leak into feature branches.

**Why it happens**

- hidden planner refresh still begins from tracked planning files
- leased worktrees are normal git worktrees on agent branches

**Failure scenario**

- agent slot worktree updates planning files during completion flow
- the agent branch now carries unrelated planning changes
- later PR review sees planning drift mixed into implementation work

**Current guardrails**

- reconciliation can restore some protected files after a turn

**Remaining gap**

- the wrong file was still the authority source to begin with

**Target response**

- make tracked planning files import/export only and move authority to repo-shared runtime storage

**Code references**

- `src/adapter/outbound/filesystem_planning_workspace_adapter.rs`
- `src/application/service/planning_reconciliation_service.rs`
- `src/adapter/inbound/tui/app/turn_submission_runtime/stream_execution.rs`

## R7. Restart Recovery Gap

**Problem**

Some runtime state survives in files, but ordering, claims, and global delivery coordination do not.

**Why it happens**

- pool state is partially durable
- refresh gates and queue processing coordination are largely process memory or claim-less scans

**Failure scenario**

- app exits during official refresh or distributor delivery
- the next process sees partial runtime files but not the reserved ordering or ownership state

**Current guardrails**

- some queue and session files remain on disk
- blocked states can force manual inspection

**Remaining gap**

- no explicit recovery sweep contract
- no append-only runtime event log for replay or audit

**Target response**

- add runtime-domain event log and recovery projections in the repo-shared store

**Code references**

- `src/application/service/planning_worker_orchestration_service.rs`
- `src/application/service/parallel_mode_turn_service.rs`
- `src/application/service/parallel_mode_distributor_service.rs`

## R8. Queue Snapshot Drift

**Problem**

`queue.snapshot.json` is derived but can still drift from what runtime would derive from the current
ledger.

**Why it happens**

- runtime builds queue from accepted ledger in memory
- exported file views are still file-backed artifacts

**Failure scenario**

- queue file is stale after interrupted reconciliation
- operator or tooling reads the file and concludes the wrong next task

**Current guardrails**

- runtime usually trusts the derived in-memory queue, not the stale exported file alone

**Remaining gap**

- exported file views are not revision-coupled to the accepted store state

**Target response**

- generate exports only from committed authority-store revisions

**Code references**

- `src/application/service/planning_prompt_service.rs`
- `src/application/service/planning_reconciliation_service.rs`

## Recommended Architectural Response

The current file-backed model should be replaced by a repo-shared planning authority store with:

- one canonical repo-scoped authority domain for planning plus runtime claims and projections
- store-backed `draft -> validate -> promote` instead of direct active-state mutation
- transactional active commits for ledger and queue projection
- separate planning revision and runtime event sequencing
- tracked planning files reduced to export and explicit import artifacts
- recovery that rechecks Git and GitHub truth instead of replay-only recovery

See [18-repo-shared-planning-authority-store.md](18-repo-shared-planning-authority-store.md) for
the detailed redesign.
