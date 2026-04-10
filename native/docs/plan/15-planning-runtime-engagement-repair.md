# Planning Runtime Engagement Repair

This plan refines `docs/design/06-direction-task-ledger-and-priority-queue.md` and follows the completed `docs/plan/14-planning-init-manual-editor-rollout.md`.

The immediate goal is not another local planning overlay fix. The goal is to remove the structural split that keeps letting planning-aware behavior work in one path and disappear in another.

Unless noted otherwise, file paths below are relative to `native/`.

## Problem Summary

The current planning flow can make the workspace look initialized while the first real turn still behaves like an unmanaged conversation.

Observed mismatch:

- `:planning` can stage, review, and promote valid planning files
- the shell footer can show planning as ready
- auto follow-up prompts can append `Planning Context`
- the first manual prompt after initialization still goes through the plain manual submit path without planning context
- builtin next-task auto follow-up can still run when the planning queue has no actionable `next_task`

This is why the same feature keeps looking fixed in one slice and broken in the next user journey.

## Structural Root Causes

### 1. Prompt assembly is split by origin instead of owned in one place

- manual submit currently forwards `conversation.input_buffer.trim()` directly into `PromptOrigin::Manual`
- auto follow-up builds its own prompt through `AutoFollowState::render_prompt(...)`
- planning context is appended only in the auto follow-up renderer

Result:

- planning engagement is origin-dependent
- fixes to planning overlays or planning file creation do not guarantee that the next manual turn will honor planning state

### 2. Planning state is represented as a prompt fragment string instead of a typed runtime contract

The runtime mostly consumes planning through `PlanningPromptContextLoadResult`, which exposes:

- `preview_status_label()`
- `preview_detail()`
- `prompt_fragment()`
- `blocks_auto_followup()`

That is enough for display and string appending, but not enough for correct execution policy.

Result:

- the code cannot distinguish "planning files are valid" from "planning queue has executable work"
- queue state such as `next_task: none` is buried inside prompt text instead of a typed readiness field

### 3. `Ready` means "valid files", not "actionable planning execution"

`PlanningPromptContextLoadResult::Ready` currently means:

- all planning files exist
- validation passed

It does not mean:

- the queue has a next task
- builtin next-task follow-up is semantically allowed
- manual turns should be forced into planning-aware prompting

Result:

- the shell reports planning as ready while execution policy is still ambiguous

### 4. Auto follow-up template policy is detached from planning queue semantics

The builtin next-task follow-up is allowed whenever auto follow-up is enabled and planning is not blocked.

There is no separate gate for:

- `queue has no next task`
- `current template requires a queue head`

Result:

- the runtime can generate a "next task" follow-up prompt even when the planning queue is empty

### 5. The test seam is too local

Recent fixes were validated mainly around:

- overlay entry
- staged draft editing
- promote flow

The missing end-to-end journeys are:

- `:planning -> promote -> first manual prompt`
- `:planning -> queue empty -> auto follow-up decision`
- `manual prompt + active planning` vs `auto prompt + active planning`

Result:

- local fixes keep passing while operator-visible flow still diverges

## Target Structure

### A. Introduce a typed planning runtime snapshot

Add a typed planning runtime contract that separates workspace validity from execution readiness.

Suggested shape:

- `PlanningRuntimeSnapshot`
- `workspace_status: Uninitialized | Invalid | ReadyNoTask | ReadyWithTask`
- `queue_head: Option<PlanningQueueHead>`
- `direction_summary`
- `task_ledger_mutation_contract`
- `display_prompt_fragment`

Rules:

- UI may keep using `display_prompt_fragment`
- runtime policy must stop depending on string parsing or "ready means everything is fine"

### B. Centralize prompt construction for every origin

Create one prompt assembly boundary for:

- manual submit
- auto follow-up
- planning repair

Suggested seam:

- `TurnPromptAssemblyService`

Suggested entrypoints:

- `build_manual_prompt(...)`
- `build_auto_follow_prompt(...)`
- `build_planning_repair_prompt(...)`

Rules:

- manual and auto prompts must read the same planning runtime snapshot
- prompt-origin differences should be policy decisions, not duplicated ad-hoc string assembly

### C. Separate planning engagement policy from planning display state

The runtime needs a typed rule for when planning should affect execution.

Suggested policy:

- manual turns: if planning is `ReadyNoTask` or `ReadyWithTask`, attach planning contract or a reduced planning summary
- builtin next-task auto follow-up: allowed only in `ReadyWithTask`
- planning repair: stays explicit and separate

This keeps the UI honest:

- "planning is ready" is not the same claim as "next-task automation may proceed"

### D. Make template gating queue-aware

Builtin templates should declare what planning readiness they require.

Suggested rule:

- builtin `next-task` requires `ReadyWithTask`
- generic templates may allow `ReadyNoTask`

This prevents the empty-queue follow-up bug without hardcoding queue logic into unrelated templates.

### E. Add journey-level regression tests

Required coverage after the structural seam exists:

- `:planning simple -> promote -> manual prompt includes planning engagement`
- `:planning simple -> queue empty -> builtin next-task auto follow-up stops instead of queueing`
- `detail/manual editor save -> promote guidance remains explicit`
- `planning invalid -> manual and auto paths both block consistently`

## Merge Order

1. current: `docs/native-planning-runtime-engagement-plan`
   - scope: root-cause analysis, target structure, and bug note for planning/manual prompt drift
2. next: `feature/native-planning-service-runtime-snapshot`
   - scope: add typed planning runtime snapshot and separate `ReadyNoTask` from `ReadyWithTask`
3. next: `feature/native-runtime-turn-prompt-assembler`
   - scope: centralize manual, auto-follow, and repair prompt construction behind one service
4. next: `fix/native-runtime-planning-manual-submit`
   - scope: route manual submit through the new planning-aware prompt assembly boundary
5. next: `fix/native-followup-empty-queue-gate`
   - scope: stop builtin next-task follow-up when the queue has no actionable head
6. next: `fix/native-runtime-planning-journey-tests`
   - scope: add end-to-end coverage for the full `:planning -> prompt -> auto-follow` journey

## Current Slice

- branch: `docs/native-planning-runtime-engagement-plan`
- goal: document the failure mode before another local planning bug fix hides the same structural gap again
- dependency: `origin/prerelease` already contains PR `#134`
- verification:
  - doc review
  - branch and worktree posture review

## File Ownership

- planning runtime design:
  - `docs/design/06-direction-task-ledger-and-priority-queue.md`
  - `docs/plan/15-planning-runtime-engagement-repair.md`
- short bug capture:
  - `docs/plan/16-planning-runtime-engagement-bug-note.md`
- concurrency posture:
  - `docs/plan/11-parallel-worktree-plan.md`

## Scope Guard

This slice should include:

- root-cause analysis for planning/manual prompt drift
- a target architecture that removes prompt-origin divergence
- explicit follow-up slices for implementation
- a brief markdown bug note for the currently observed issue

This slice should not include:

- the runtime code fix itself
- queue policy changes without the typed runtime seam
- ad-hoc UI copy changes that try to explain away the structural mismatch

## Expected Follow-Up

The first implementation slice after this doc should land the typed runtime snapshot before touching manual submit behavior.

That order matters:

- without the typed snapshot, the next fix will likely patch only one prompt origin again
- with the typed snapshot in place, manual submit and auto follow-up can share one planning engagement policy
