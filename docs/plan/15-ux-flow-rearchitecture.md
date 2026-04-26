# UX Flow Rearchitecture

This document defines the target operator flow for the next product iteration. It chooses a simpler shell mental model and a more explicit recovery model.

## Design Goal

The shell should feel like one continuous work loop with three supporting abilities:

- inspect the current state
- author or repair planning state
- resume safely after a pause

The operator should not have to infer system internals to keep going.

## Chosen Mental Model

The shell will be documented and evolved around four top-level states.

| State | Meaning | Operator question it answers |
| --- | --- | --- |
| Prepare | startup, session restore, and readiness checks | can I begin work here? |
| Work | prompt submission, streaming, and active continuation | what is happening now? |
| Inspect | queue, automation, diagnostics, and session inspection | what does the system currently know? |
| Recover | invalid planning, paused automation, and close-risk flows | what is the safest next action? |

This is a documentation and product-language choice first. The implementation can still use finer-grained internal states.

## Chosen Information Architecture

### Conversation Surface

Owns:

- prompt entry
- live stream output
- compact status
- current-session identity

Must always answer:

- what the shell is doing right now
- whether the operator can submit
- whether internal continuation is active, paused, or stopped

### Diagnostics Overlay

Owns:

- startup blockers
- environment readiness
- re-run startup checks

Must not become a general runtime problem browser.

### Sessions Overlay

Owns:

- search
- paging
- current-workspace filtering
- session selection

Must not expose planning state directly. It should remain a session-resume tool.

### Continuation Status

Owns:

- current continuation policy
- next-turn preview
- pause reason
- explicit resume path

Must not be the only place where pause reason is visible. It is status projection, not a separate user-facing control plane.

### Queue Overlay

Owns:

- current executable task
- near-future queue
- proposed candidates
- blocked or skipped summary

It should read like a work board, not like a planning-file dump.

### Planning And Directions Flows

Own:

- authoring planning state
- staged review
- promotion
- supporting-file maintenance

They should be treated as authoring flows, not generic inspection overlays.

### Shared Lifecycle Commands

Own:

- non-interactive planning bootstrap before entering the TUI
- read-only planning health inspection before entering the TUI
- safe reset flows for queue-only, directions-only, or full planning workspace reset

The external `akra` command surface and the in-shell `:` command surface already share one lifecycle model. Future UX work should treat them as fixed entry and recovery affordances, not as a separate redesign track.

## Chosen Overlay Purpose Map

| Surface | Primary job | Secondary job | Should avoid |
| --- | --- | --- | --- |
| main shell | work loop | compact state summary | verbose troubleshooting |
| diagnostics | readiness | restart checks | planning repair |
| sessions | resume context | search past work | queue details |
| automation | policy and pause control | next-turn preview | raw planning semantics |
| queue | work explanation | proposal visibility | authoring |
| planning | setup and editing | promote accepted state | runtime debugging |
| directions | supporting planning maintenance | create detail docs and queue-idle prompt | full queue inspection |
| doctor/init/reset commands | workspace lifecycle management | safe entry and recovery before or during interactive use | replacing the main authoring and inspection flows |

## Canonical Journeys

### 1. First Run To First Turn

Target flow:

1. Shell opens and startup diagnostics begin.
2. The operator can type immediately.
3. The shell visibly distinguishes "typed" from "ready to send."
4. Once startup becomes ready, the queued prompt submits automatically or clearly invites submit.
5. The operator sees one compact status explanation, not multiple competing readiness messages.

Required UX qualities:

- readiness state must be obvious
- blocked state must name the blocking subsystem
- diagnostics should feel optional once ready, not mandatory to keep open

### 2. Resume Existing Session

Target flow:

1. Operator opens sessions.
2. Operator filters and opens a prior thread.
3. Shell restores conversation context and current-work context together.
4. The operator can immediately tell whether planning is active for this workspace and whether continuation can proceed.

Required UX qualities:

- session restore should not hide planning state
- resumed shell should surface current queue summary if planning is active
- the operator should not need to open three overlays to understand what remains

### 3. Planning Setup To First Auto-Follow

Target flow:

1. Operator opens `:planning`.
2. Simple mode is presented as the fastest safe path.
3. Staged review focuses on what will become active, not on raw artifact count.
4. Promotion makes planning active and explains what queue behavior becomes possible.
5. After the next turn, queue-driven continuation either runs or explains why it paused.

Required UX qualities:

- first-use planning must feel like enabling continuity, not filling out a framework
- queue-idle policy should be understandable without reading file names
- auto-follow preview should read like "the next contracted turn"

### 4. Automation Pause And Recovery

Target flow:

1. A turn completes.
2. Automation either continues, stops, or pauses.
3. If it pauses, the shell states:
   current state, reason, next action
4. The relevant surface is obvious:
   diagnostics, planning, queue, or automation
5. The operator can recover without reverse-engineering internal guards.

Required UX qualities:

- paused state must feel recoverable
- system language must translate into operator action
- planner debug detail must stay optional

## Status Language Standard

All operator-facing pause and state messages should be documented and later refactored toward this pattern:

- current state
- cause
- next action

Preferred vocabulary:

- `ready`
- `waiting`
- `running`
- `paused`
- `blocked`
- `repairing`
- `review needed`

Avoid exposing raw implementation terms unless the operator can act on them directly.

For example:

- prefer "planning is valid but has no next task" over "queue head missing"
- prefer "automation paused because the queue did not advance" over "repeated queue head"
- prefer "planning needs repair before automation can continue" over "invalid planning snapshot"

## Copy And Interaction Defaults

- The main shell should always carry the shortest trustworthy summary.
- The matching overlay should carry the full explanation.
- Recovery actions should be named as verbs:
  reopen planning, review queue, rerun diagnostics, resume manually.
- Lifecycle commands should also be verbs:
  initialize workspace, inspect health, reset queue, reset directions.
- Expert detail such as planner debug output should remain behind explicit toggles.

## Acceptance Criteria

- Every pause state maps to exactly one primary recovery surface.
- Queue and planning surfaces use the same vocabulary for task, proposal, blocked, paused, and next action.
- A first-time power user can infer the next safe action from the shell without opening source files or docs.
- Overlay purpose is distinct enough that the operator rarely needs to open more than one overlay to understand a single problem.
