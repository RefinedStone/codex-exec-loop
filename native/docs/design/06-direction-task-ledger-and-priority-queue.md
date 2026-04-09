# Direction, Task Ledger, And Priority Queue Design

This document defines the next large workstream for the native client: move auto follow-up from simple template chaining into planning-aware task orchestration.

## Why This Workstream Exists

The current product can only choose a follow-up prompt template and send one more turn. That is useful, but it does not provide:

- a durable direction catalog that defines the operator's macro intent
- a task ledger that can be revised by the operator or the LLM
- a runtime priority queue that picks the next executable task across all directions
- validation and repair when the LLM writes an invalid planning file
- rollback when the LLM touches files that must remain locked during automated execution

The new design has to add those capabilities without breaking the current shell-first `codex app-server` baseline.

## Current-State Baseline

The implemented baseline matters because this work should extend the product, not restart it.

- follow-up behavior today is driven by `FollowupTemplateService` and `AutoFollowState`
- follow-up templates are plain text bodies loaded from builtins plus `.codex-exec-loop/followups/*.md|*.txt`
- prompt generation happens after turn completion in the conversation runtime
- stop rules already exist for explicit stop-keyword matches and no-file-change turns
- the runtime currently tracks file-change count and a compact text summary, but not a structured list of changed file paths in domain state
- the shell already has inspection surfaces and warnings, so planning status should be surfaced there instead of inventing a separate runtime UI

This means the new planning system should sit beside the existing follow-up flow and progressively upgrade it.

## Terminology

- `direction`: a high-level operator-owned workstream skeleton item; no priority ordering
- `task`: an executable unit of work linked to exactly one direction
- `task ledger`: the canonical structured file that stores tasks
- `priority queue`: the in-memory runtime structure derived from the accepted task ledger
- `queue snapshot`: a generated file that mirrors the in-memory queue for inspection; never authoritative
- `repair loop`: the bounded retry flow that asks the LLM to rewrite an invalid task-ledger update
- `simple mode`: the planning entry path that emits one generic catch-all direction and an empty task ledger
- `detail mode`: the planning entry path for explicit direction authoring, manual editing, and future LLM-assisted drafting
- `manual planning editor`: an embedded terminal editing surface for staged planning drafts

## Target Operating Model

The system should behave as follows:

1. The operator enters planning mode through `:planning` or edits planning files directly on disk.
2. `:planning` first asks whether the operator wants `simple mode` or `detail mode`.
3. `simple mode` stages a minimal scaffold that keeps one generic active direction and an empty task ledger.
4. `detail mode` asks whether the operator wants `manual` authoring or future `llm-assisted` authoring; until implemented, the LLM-assisted branch stays visible but disabled.
5. The operator reviews and promotes staged planning files into the active planning paths.
6. The runtime validates the direction catalog and task ledger before each planning-aware turn.
7. The runtime rebuilds an in-memory priority queue from the accepted task ledger.
8. The selected follow-up template injects direction context, queue context, mutation rules, and result-output rules into the next prompt.
9. The LLM performs the selected work and may update the task ledger only with tasks that can be linked to an existing direction.
10. After the turn, the runtime validates the changed task ledger.
11. If the ledger is valid, the runtime accepts it and rebuilds the queue.
12. If the ledger is invalid, the runtime restores the last accepted ledger, stores the rejected attempt for inspection, and runs a bounded repair loop.
13. If the LLM touched an execution-locked planning file, the runtime restores the operator-owned snapshot and records the violation.

## Canonical Sources Of Truth

The design must remove ambiguity about which state is authoritative.

### 1. Protected Direction Catalog

Suggested path:

- `.codex-exec-loop/planning/directions.toml`

Properties:

- authoritative for high-level direction structure
- user-owned
- readable by the runtime and the LLM
- easy to read and edit directly as a file
- editable by the operator while the runtime is idle or paused
- not writable by the LLM during active automated execution
- no queue priority fields
- still required in `simple mode`; the file uses one generic catch-all direction instead of a detailed taxonomy

Why TOML:

- easier for operators to edit than raw JSON
- strict enough for validation and stable parsing in Rust
- supports comments without inventing a markdown parser contract
- fits the v1 requirement that direct file editing is the primary authoring path

### 2. Shared Task Ledger

Suggested path:

- `.codex-exec-loop/planning/task-ledger.json`

Properties:

- authoritative for executable tasks
- writable by the operator
- writable by the LLM, but only if the result passes validation
- every task must reference an existing `direction_id`
- priority lives here, not in the direction catalog

Why JSON:

- stricter than markdown or YAML for LLM writes
- straightforward schema validation
- less ambiguity during repair prompts

### 3. Derived Queue Snapshot

Suggested path:

- `.codex-exec-loop/planning/queue.snapshot.json`

Properties:

- generated by the runtime from the accepted task ledger
- read-only to both user prompts and LLM instructions
- visible for debugging, inspection, and restart continuity
- never the source of truth

This decision is important. The queue file must not be edited directly, otherwise the file state and the in-memory queue will diverge.

### 4. Result Prompt Fragment

Suggested path:

- `.codex-exec-loop/planning/result-output.md`

Properties:

- operator-owned prompt fragment
- injected by Rust into the outbound prompt
- not parsed as a planning data file
- can evolve independently from queue or ledger logic

### 5. Validation Contract

Suggested path:

- `.codex-exec-loop/planning/task-ledger.schema.json`

Properties:

- machine-readable schema for the task ledger
- used by runtime validation
- referenced when the runtime asks the LLM to repair an invalid ledger

## Required Invariants

These rules close the main logical holes in the proposed workflow.

1. The direction catalog is immutable to the LLM during active automated execution. If the LLM modifies it, the runtime restores the pre-turn snapshot.
2. Directions do not carry execution priority. Execution order is determined only by queued tasks.
3. Every queued task maps to exactly one existing `direction_id`.
4. The queue is derived from the accepted task ledger and never accepted from direct file edits.
5. A task-ledger update becomes authoritative only after schema validation and business-rule validation both pass.
6. Invalid task-ledger updates do not advance auto follow-up.
7. User edits to protected planning files are allowed only while the system is paused or idle.
8. A new LLM-authored task may enter the ledger only if it can be attached to an existing direction with an explicit relation note.
9. Completely unrelated tasks must not be added to the task ledger.
10. Runtime restart must be able to rebuild the same queue from accepted on-disk planning files without replaying prior turns.
11. `simple mode` does not remove the direction catalog; it replaces detailed direction authoring with one generic active direction so task-ledger invariants stay uniform.

## Minimal Data Model

The exact Rust structs can change, but the logical model should stay stable.

### Direction Catalog

Each direction item should contain:

- `id`
- `title`
- `summary`
- `success_criteria`
- `scope_hints`
- `state` with values such as `active`, `paused`, `done`

Directions are intentionally simple. They are the operator's declared frame, not a scheduler.

In `simple mode`, the catalog should contain exactly one generic active direction such as `general-workstream` so every task can still satisfy the `direction_id` contract without forcing the operator to define a richer hierarchy up front.

### Task Ledger

Each task should contain:

- `id`
- `direction_id`
- `direction_relation_note`
- `title`
- `description`
- `status`
- `base_priority`
- `dynamic_priority_delta`
- `priority_reason`
- `depends_on`
- `blocked_by`
- `created_by`
- `last_updated_by`
- `source_turn_id`
- `updated_at`

Recommended task statuses:

- `ready`
- `blocked`
- `in_progress`
- `done`
- `cancelled`
- `awaiting_user`
- `proposed`

`base_priority` is the stable baseline. `dynamic_priority_delta` is the runtime-or-LLM adjustment layer. The queue order is built from both values so operator intent stays visible even when the LLM reprioritizes locally.

`direction_relation_note` is required for LLM-added tasks. The note explains why the task belongs under the chosen direction. The relation can be loose, but it cannot be empty.

`proposed` does not mean "unattached idea". It means a direction-linked task candidate that is not yet ready for normal execution.

In `simple mode`, every task points at the one generic direction until the operator later replaces the scaffold with a more detailed direction catalog.

## Task Admission Rule

The task ledger does not accept free-floating proposals.

- if a follow-up item can be connected to an existing direction, it should be written as a normal task under that direction
- if the LLM cannot reasonably connect the item to any existing direction, it must not write that item into the ledger
- the runtime enforces the structural part of this rule by requiring `direction_id` and `direction_relation_note`
- semantic relatedness remains intentionally loose, so the initial runtime should not try to over-police meaning beyond that contract

This keeps the ledger focused on work that is still inside the operator's declared frame.

## Queue Semantics

The queue is global across all directions.

- the runtime does not execute one direction from start to finish by default
- the next task is selected from the full set of executable tasks
- blocked, done, cancelled, and awaiting-user tasks are excluded from the active queue

Recommended ordering tuple:

1. manual pin or operator override
2. task readiness
3. combined priority score: `base_priority + dynamic_priority_delta`
4. dependency completeness
5. oldest `updated_at` among otherwise-equal items to reduce starvation

The queue should expose both:

- the next executable task
- the top N visible tasks with the reasons they are ranked that way

That second surface is important because the operator needs to understand why the system chose a task.

## Planning State Window

Planning behavior should differ by runtime state.

- `uninitialized`: no planning files yet; current simple follow-up mode still works
- `authoring`: no turn is running; the operator may directly edit planning files or run the planning-init helper
- `ready`: planning files are valid and the queue is ready, but no turn is running
- `executing`: a turn is active; execution-locked files are snapshotted and protected
- `repairing`: the runtime is trying to recover from an invalid task-ledger write
- `blocked_invalid`: planning mode cannot continue until the operator fixes invalid files

This state split closes the ambiguity around "can the user edit this file right now?".

## File Ownership And Mutation Rules

### User-Owned Files

- `directions.toml`
- `result-output.md`

Runtime behavior:

- direct file editing is the primary v1 authoring path
- user edits are allowed while the runtime is idle or paused
- snapshot before each automated turn
- restore if the LLM changed them during execution
- show a warning in the shell if a rollback occurred

### Shared Files

- `task-ledger.json`

Runtime behavior:

- allow user edits while idle or paused
- allow LLM edits during a turn
- accept only after validation
- if invalid, restore the last accepted revision and enter repair mode

### Generated Files

- `queue.snapshot.json`

Runtime behavior:

- rebuild only from accepted task-ledger revisions
- overwrite freely
- do not accept direct user or LLM edits as meaningful input

## Validation Pipeline

Validation should happen in three layers.

### 1. Parse And Schema Validation

Check:

- file exists when planning mode is enabled
- JSON and TOML parse correctly
- required fields exist
- field types and enum values are valid

### 2. Cross-Reference Validation

Check:

- every task `direction_id` exists
- `depends_on` ids exist
- no duplicate task ids or direction ids
- no dependency cycles among executable tasks

### 3. Policy Validation

Check:

- protected fields were not changed by the wrong actor
- statuses and priorities remain inside allowed transitions
- direction mutations from the LLM are rejected
- new LLM-authored tasks include both `direction_id` and `direction_relation_note`
- tasks referencing a missing direction are rejected
- queue snapshot is regenerated, not user-authored

If any layer fails, the runtime should emit a structured validation report that can be shown in the shell and reused in repair prompts.

## Planning Initialization And Authoring Assistance

The product should help the operator produce valid planning files before automated execution starts.

### Primary V1 Authoring Path

- `:planning` is the primary guided entrypoint inside the shell
- direct filesystem editing remains supported for advanced users and scripts
- every guided path stages files first and requires an explicit promote step before active planning changes

### `:planning` Entry Flow

The `:planning` command should open a lightweight selector instead of immediately writing one fixed scaffold.

Recommended selector behavior:

1. Show `simple mode` and `detail mode` as the first two options.
2. Let the operator move between them with `A` / `B`, arrow keys, or equivalent focus movement, and confirm with `Enter`.
3. Allow `Esc` to cancel without writing files.
4. Keep the UI lightweight and shell-native rather than opening a heavyweight planning dashboard.

### Simple Mode

`simple mode` is for operators who want planning-aware execution without investing in explicit direction taxonomy first.

Recommended behavior:

1. Stage a minimal valid planning scaffold under `.codex-exec-loop/planning/drafts/`.
2. Write `directions.toml` with one generic active direction such as `general-workstream`.
3. Write `task-ledger.json` with an empty `tasks` list.
4. Keep `task-ledger.schema.json` and `result-output.md` on standard defaults.
5. Phrase the generic direction so the LLM can interpret it as "put all actionable goals or accepted proposals into the task ledger and work from there."
6. Allow the operator to later replace the generic direction catalog with a richer detail-mode catalog without changing the task-ledger storage shape.

### Assisted Initialization Flow

`detail mode` should branch again after the first selector:

Suggested operator entrypoints:

- `:planning` in the TUI
- `:planning-init` as a compatibility alias
- a future `/init` alias if a slash-command surface is introduced

Detail-mode selector behavior:

1. Show `manual` and `llm-assisted` as the second-level options.
2. Keep `llm-assisted` visible but disabled until supported.
3. Make `manual` the only selectable implementation for the first delivery.

### Manual Detail Mode

`manual` detail mode should not stop at scaffold creation. It should let the operator keep editing the staged files in the terminal.

Recommended behavior:

1. Stage a detail scaffold under `.codex-exec-loop/planning/drafts/`.
2. Open an embedded terminal editor over the staged files.
3. Focus editing on `directions.toml`, `task-ledger.json`, and `result-output.md`.
4. Keep `task-ledger.schema.json` read-only by default, or hide it behind an advanced inspection toggle.
5. Provide explicit `save`, `validate`, `cancel`, and `promote` actions.
6. Show validation errors inline and keep `promote` disabled until the staged files validate.
7. Leave the staged files on disk if the operator exits without promoting, so the draft can be resumed later.

### Future LLM-Assisted Detail Mode

The LLM-assisted branch is still part of the target design, but it should come after the manual path is stable.

Recommended future behavior:

1. Collect operator input such as goal summary, direction ideas, constraints, and non-goals.
2. Open a dedicated planning-init thread with a fixed prompt contract.
3. Ask the LLM to draft `directions.toml` and `task-ledger.json` only.
4. Write the generated files into the same staging area as manual mode.
5. Run the same validation and promote flow as manual mode.

This preserves one safe authoring lifecycle across both detail-mode branches and keeps active planning state operator-approved.

## Runtime Lifecycle

### Startup

On startup or workspace switch:

1. Load the direction catalog, task ledger, and result prompt fragment.
2. Validate direction catalog and task ledger.
3. Build the in-memory priority queue from the accepted ledger.
4. Publish planning warnings into the existing shell warning/notices channel.

If planning files are missing, the product should not crash. It should remain in today's basic follow-up mode until the workspace is initialized.

### Pre-Turn Gate

Before any planning-aware prompt is sent:

1. Detect whether planning files changed since the last accepted digests.
2. Re-validate if they changed.
3. Rebuild the queue if validation passed.
4. Refuse planning-aware auto follow-up if validation failed.

This gate must run before prompt assembly, not after, because the prompt content depends on accepted planning state.

### Prompt Assembly

The prompt should become a composition of:

1. the selected follow-up template body
2. a runtime-injected direction summary block
3. a runtime-injected queue block
4. a strict mutation contract for `task-ledger.json`
5. an explicit rule that new tasks must attach to an existing `direction_id` with a relation note, and unrelated items must not be written
6. an explicit prohibition on editing execution-locked files
7. the injected contents of `result-output.md`

The existing template catalog still matters. The template remains the operator's steering choice, but it now sits on top of validated planning context instead of acting alone.

### Turn Guard

Before the stream starts:

- snapshot protected files
- capture accepted digests for the task ledger
- mark planning state as `executing`

### Post-Turn Reconciliation

After the turn completes:

1. Inspect which planning files changed.
2. If `directions.toml` changed, restore it and emit a protected-file rollback notice.
3. If `task-ledger.json` changed, validate it.
4. If valid, accept it, rebuild the queue, and refresh the generated queue snapshot.
5. If invalid, move the rejected content into a rejection archive, restore the last accepted ledger, and schedule a repair prompt if retry budget remains.

## Repair Loop

The repair loop must be explicit and bounded.

Recommended behavior:

1. Save the invalid candidate into `.codex-exec-loop/planning/rejected/`.
2. Restore the last accepted `task-ledger.json`.
3. Emit a repair prompt that includes:
   - the validation errors
   - the allowed schema contract
   - the rejected candidate excerpt or diff
4. Allow at most `N` repair retries for the same failed turn, with `N` small, such as `2` or `3`.
5. If repair still fails, disable auto follow-up and require operator intervention.

The repair flow should not silently discard LLM intent. Rejected content should stay inspectable.

## Handling Runtime Priority Changes

The design should acknowledge that priority can change during real work.

- the operator owns the direction frame
- the operator sets or adjusts stable task priority through `base_priority`
- the LLM may adjust `dynamic_priority_delta` with a required `priority_reason`
- the queue ordering uses the combined score, not the delta alone

This split avoids two bad outcomes:

- the LLM cannot erase original operator intent
- the operator still sees why a task moved up or down

## Integration With The Current Hexagonal Architecture

This work fits the existing layers if boundaries remain clear.

### Domain

Add UI-neutral models for:

- direction catalog
- task ledger
- queue entry
- planning validation report
- protected-file violation
- planning workspace state

### Application Services

Introduce planning-focused services such as:

- `PlanningBootstrapService`
- `PlanningWorkspaceService`
- `PlanningValidationService`
- `PriorityQueueService`
- `PlanningPromptAssemblyService`
- `PlanningReconciliationService`

These services should orchestrate planning logic without owning file I/O or TUI state.

### Outbound Ports And Adapters

Add ports for:

- reading and writing planning files
- snapshot and restore of protected files
- persisting rejected task-ledger candidates

Important current-state gap:

- the runtime will need structured changed-file paths, not only the current file-change count and summary text

That means the outbound `codex app-server` mapping and conversation domain events should be extended to carry changed paths when available.

### Inbound TUI

Add operator-visible planning signals without breaking the current shell shape:

- a lightweight `:planning` selector for `simple mode` and `detail mode`
- a second-level selector for `manual` and future `llm-assisted` detail authoring
- an embedded manual planning editor for staged drafts, validation results, and explicit promote actions
- planning status: valid, stale, invalid, repairing
- selected queue-head task
- last validation failure summary
- protected-file rollback notice

This should still feel like an extension of the current shell surfaces, not a separate heavyweight planning product inside the TUI.

## Backward Compatibility

The current simple follow-up mode should remain usable.

- if a workspace has no planning files, current builtin follow-up templates still work
- the operator may enter authoring mode, edit files, validate them, and then return to automated execution
- `simple mode` should cover the low-ceremony case where the operator does not want to define named directions yet
- direct filesystem editing remains valid even after the guided `:planning` flow exists
- planning-aware templates should activate only when the workspace is initialized
- stop-keyword and no-file-change rules remain valid and should continue to gate auto follow-up

This keeps rollout incremental and avoids blocking today's users.

## Recommended Initialization Template

Planning initialization should stage:

- `.codex-exec-loop/planning/directions.toml`
- `.codex-exec-loop/planning/task-ledger.json`
- `.codex-exec-loop/planning/task-ledger.schema.json`
- `.codex-exec-loop/planning/result-output.md`

Mode-specific expectations:

- `simple mode`: `directions.toml` contains one generic active direction and `task-ledger.json` starts empty
- `detail mode`: `directions.toml` is intended for explicit direction authoring and `task-ledger.json` starts from a richer operator-edited draft
- `queue.snapshot.json`: generated only after the staged planning files are promoted and accepted by runtime validation

## Explicitly Rejected Alternatives

### Queue File As Source Of Truth

Rejected because:

- it creates split-brain state against the runtime queue
- it complicates restart consistency
- it makes validation and repair ambiguous

### Letting The LLM Edit Directions Directly

Rejected because:

- the direction catalog is the operator's strategic frame
- the LLM should only work within the current direction frame during automated execution
- rollback is far easier than trying to determine whether an LLM direction edit was legitimate

### Markdown As The Canonical Task Ledger

Rejected because:

- free-form markdown is too weak as a mutation contract
- invalid formatting repair becomes under-specified
- strict queue rebuilding depends on predictable structure

## Delivery Sequence

Recommended implementation order:

1. Add planning domain models plus file contracts.
2. Add `:planning` mode selection plus simple-mode generic scaffold creation.
3. Add detail-mode manual editor, staged validation, and explicit promote flow.
4. Add validation and queue rebuild services.
5. Extend prompt assembly with planning context injection and task-admission rules.
6. Extend runtime events so changed planning-file paths are available after each turn.
7. Add protected-file rollback and task-ledger repair flow.
8. Surface planning status inside the shell.
9. Add the future detail-mode LLM-assisted drafting branch after the manual path is stable.

This order minimizes risk because validation and source-of-truth rules land before auto-repair and UI polish.

## Success Criteria

The work is successful when all of the following are true:

- the operator can define directions without giving the LLM authority to rewrite them
- the LLM can append or revise tasks in a strict format
- invalid task updates are detected before they corrupt runtime planning state
- the in-memory priority queue is always rebuildable from accepted files
- the next auto-follow-up task is chosen from queue priority, not direction order
- the shell explains why the system continued, stopped, repaired, or rolled back planning state
