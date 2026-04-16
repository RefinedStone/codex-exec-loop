# Supersession Product Model

This document defines the target supersession model, not shipped behavior.

## Design Goal

Parallel mode should let one operator coordinate several implementation-capable agent sessions
from one shell without losing planning authority, git hygiene, or integration control.

## Core Concepts

| Term | Meaning |
| --- | --- |
| parallel mode | operating mode where the shell runs a supervisor instead of a normal single main session |
| super session | the control-tower shell surface for assignment, status, merge queue, and recovery |
| agent session | a main-grade codex/app-server session that owns one task and one leased worktree slot |
| hidden planning worker | the planning-only helper that refreshes `task-ledger.json` after agent milestones |
| task ledger | `.codex-exec-loop/planning/task-ledger.json`, the official task source of truth |
| akra pool | the managed set of git worktree slots available to agent sessions |
| slot | one reusable worktree lease that can be assigned to exactly one live agent at a time |
| distributor | the serial integration subsystem that pushes, opens PRs, merges into `akra`, and cleans slots |
| merge queue | ordered list of commit-ready agent outputs waiting for distributor integration |
| commit ready | agent completion milestone: local edits, validation, and commit are finished |

## System Relationship

The target control flow is:

1. supervisor loads capability state and pool state
2. supervisor reads official queue state from `task-ledger.json`
3. supervisor assigns a ready task to an idle slot and launches an agent session
4. agent works in its leased worktree and reports commit-ready completion
5. supervisor records the report and passes the result to hidden planning worker
6. hidden planning worker updates `task-ledger.json`
7. distributor pulls commit-ready reports from merge queue and integrates them into `akra`
8. cleaned slots return to the pool for future assignment

## Why The Execution Unit Is An Agent

The current hidden planning worker is intentionally narrow:

- it edits planning control files only
- it exists to refresh or repair planning state
- it is not a substitute for a main implementation session

The parallel-mode execution unit must be an agent because:

- it performs the same category of work as today's main session
- it can produce implementation results, validation output, and commit-ready changes
- its output must later be interpreted by hidden planning worker to refresh official queue state

The planning worker remains in the loop, but it is downstream of agents rather than replacing them.

## Normal Mode Versus Parallel Mode

| Dimension | Normal mode | Parallel mode |
| --- | --- | --- |
| primary shell identity | one main conversation | one control-tower supervisor |
| execution unit | current main session | multiple agent sessions |
| planning feedback | main reply can trigger planning refresh | agent completion report triggers planning refresh |
| queue authority | `task-ledger.json` with hidden planning worker support | same authority, but fed by multiple agents |
| git model | current workspace only | `akra` plus leased worktree pool |
| integration | operator-driven | distributor-driven merge queue |
| sessions overlay | resume existing conversations | supersession board replaces session browser entrypoint |

## Closed V1 Invariants

- Official task state changes only after hidden planning worker updates the ledger.
- Supervisor never bypasses the ledger when choosing new executable work.
- Agent sessions never write the ledger directly.
- Supervisor is operational, not conversational.
- Distributor is the only component that merges to `akra`.
- Slot reuse is allowed only after the worktree returns to clean `akra` state.
- Pool exhaustion blocks new agent creation rather than overcommitting the same slot.

## Control Loops

### Assignment Loop

- read ledger-ready work
- find idle slot
- launch agent session
- record assignment

### Planning Loop

- wait for agent milestone
- capture summary, commit info, validation info
- refresh ledger through hidden planning worker
- accept new official queue state

### Integration Loop

- enqueue commit-ready report
- run distributor on queue head
- update `akra`
- clean slot

## Boundary With Existing Product Identity

Supersession is an additive operating mode. It does not redefine current shipped single-session
behavior until the new mode is implemented and validated. Current design docs remain the canonical
description of shipped behavior.

## Related Docs

- [README.md](README.md)
- [02-operator-mode-and-shell-model.md](02-operator-mode-and-shell-model.md)
- [04-task-ledger-feedback-loop.md](04-task-ledger-feedback-loop.md)
- [../design/01-current-product-state.md](../design/01-current-product-state.md)

## Code Impact

Expected entrypoints:

- `src/application/service/session_service.rs`
- `src/application/service/planning_worker_orchestration_service.rs`
- `src/adapter/outbound/app_server_planning_worker_adapter.rs`
