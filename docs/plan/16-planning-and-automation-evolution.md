# Planning And Automation Evolution

This document chooses how planning, queue inspection, and post-turn automation should evolve in the next product stage.

## Desired Outcome

Planning should become the lightest reliable way to keep a workspace moving across multiple turns.

Automation should feel trustworthy because it is understandable, not because it is opaque and powerful.

## Chosen Defaults

- Planning continues to be file-backed and operator-owned.
- Simple mode becomes the main onboarding path and should be treated as the default narrative.
- Detail mode remains for power users and deliberate authoring.
- `llm-assisted` planning stays out of the active roadmap until manual and simple mode are clearly successful.
- Queue-driven automation stays enabled only through accepted planning state.

## Operator Vocabulary To Standardize

The product should consistently distinguish these concepts:

| Term | Meaning |
| --- | --- |
| direction | a long-lived workstream or objective |
| queue task | executable work the runtime may act on now |
| proposed task | follow-up work worth keeping visible but not yet executable |
| queue-idle policy | what the runtime does when valid planning has no actionable task |
| staged draft | inactive planning edits awaiting validation and promotion |
| accepted planning | the active planning contract the runtime trusts |
| repair | bounded system attempt to restore planning validity after invalid writes |

## Chosen Evolution Themes

### 1. Lower The Cost Of First Planning Success

The first success case should be:

- create simple planning
- promote it
- run one turn
- see a justified next action or a clear explanation of why none exists

To support that:

- simple mode should become the dominant narrative in docs and product language
- staged review should emphasize outcome, not file mechanics
- queue-idle behavior must be explained in operator language before promotion

### 2. Make Queue State Read Like Work State

The queue surface should evolve toward four stable buckets:

- now
- next
- candidates
- blocked or skipped

This is not a schema change requirement first. It is a presentation and mental-model requirement.

The operator should be able to answer:

- what can run now
- what might run next
- what is optional
- what is prevented and why

### 3. Make Automation Pause Reasons Actionable

The automation surface should explicitly separate:

- policy
- next-turn preview
- current pause reason
- resume path

The target is not to remove all pauses. The target is to make each pause predictable and recoverable.

Priority pause categories:

- planning invalid or incomplete
- queue idle by policy
- queue did not advance
- stop rule matched
- manual review required

Each category should map to one dominant operator action.

### 4. Make Repair Feel Protective, Not Mysterious

Repair and rollback are strengths of the product, but they are currently easy to read as failure instead of protection.

The next iteration should document and eventually surface repair as:

- what changed
- what was rejected
- what the system restored
- what the operator should inspect next

### 5. Tighten The Relationship Between Resume And Planning

When a workspace already has accepted planning, resumed conversation should feel like resumed work management as well.

The operator should not need to rediscover:

- whether planning is active
- whether the queue is actionable
- whether proposals remain
- whether automation is currently safe to continue

## Planned Capability Changes

### Planning Authoring

- keep simple mode as the fast path
- keep detail mode as the advanced path
- make directions maintenance clearly subordinate to planning authoring rather than a parallel system
- document queue-idle prompt authoring as an advanced customization path

### Workspace Lifecycle Commands

- add `akra doctor` as a non-interactive planning health check for the current workspace before the TUI starts
- add `akra init` as a non-interactive planning bootstrap command that defaults to the simple scaffold
- add `akra reset` as a safe reset command that can target queue-only, directions-only, or the full planning workspace
- add in-shell counterparts `:doctor`, `:init`, and `:reset`
- keep command semantics aligned with planning authoring and validation rules instead of creating a second planning model

### Queue Inspection

- show executable work before structural metadata
- elevate proposals as visible candidates rather than hidden planner leftovers
- summarize skipped or blocked work in one predictable section

### Automation Controls

- keep policy editing and preview together
- make pause explanation mandatory when automation does not continue
- make debug detail optional and non-blocking

### Future Expansion, Not Immediate Scope

- guided direction scaffolding
- more explicit proposal-promotion affordances
- approval review actions once upstream protocol and client affordances are ready

## Phased Delivery

### Phase 1

- document the operator vocabulary
- simplify current explanations of queue and pause state
- reframe repair and queue-idle behavior in the docs and UI copy
- define the command contract for `akra doctor`, `akra init`, `akra reset`, and their in-shell counterparts

### Phase 2

- tighten first-run planning flow around simple mode
- improve queue overlay semantics and proposal visibility
- reduce the number of places an operator must visit to recover from a paused automation cycle
- implement workspace lifecycle commands on top of the same planning bootstrap, validation, and reset rules used by the TUI

### Phase 3

- deepen continuity between resumed sessions and accepted planning state
- revisit advanced authoring and approval workflows only after the baseline loop is easy to trust

## Acceptance Criteria

- A power user can enable planning and understand the first queue-driven result without reading the planning file set in raw form.
- Queue inspection visibly distinguishes executable work from optional candidates.
- Automation pause states always point to one next recovery action.
- Repair flow reads as a safe restoration mechanism rather than an obscure planner failure.

## Explicit Non-Goals

- making planning invisible
- replacing file-backed planning with hidden runtime-only state
- turning proposals into automatically executing tasks by default
- shipping `llm-assisted` planning because it is visible in the UI
