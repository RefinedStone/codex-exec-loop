# Product Elevation Blueprint

This document is the execution blueprint for the next stage of `codex-exec-loop`. It is intentionally future-facing and should guide implementation planning rather than describe shipped behavior.

## Product Thesis

`codex-exec-loop` should evolve from "a TUI client with planning features" into "a personal execution cockpit for Codex-driven long-running work."

The target user is a power solo developer who:

- stays in one terminal for long stretches
- revisits the same workspace repeatedly
- wants queue-driven continuity instead of manually deciding every next step
- accepts structured planning files if the payoff is lower cognitive overhead and more reliable execution

## Why Elevation Is Needed Now

The current product already has strong primitives:

- inline scrollback shell
- startup diagnostics
- recent session resume
- planning workspace with staged promotion
- queue-driven auto follow-up
- repair and rollback protections

The limiting factor is no longer raw capability. It is product coherence.

Today the product feels more advanced than it feels clear. The operator must understand startup state, queue state, proposal state, draft state, repair state, and follow-up state at the same time. That is the main bottleneck to deeper adoption.

## Current Structural Problems

| Problem | Current symptom | Product cost |
| --- | --- | --- |
| Weak mental model | many overlays and status phrases expose implementation state directly | the operator has to learn the system instead of using it |
| Planning-first power with planning-first friction | planning becomes valuable only after non-trivial setup | new workspaces feel expensive to prepare |
| Automation is strong but not self-explanatory | accurate pause reasons still read like internals | trust in auto follow-up grows slower than capability |
| Surface roles overlap | planning, queue, automation, and directions each explain part of the same truth | recovery actions are not obvious |
| Large runtime surfaces | shell and planning behavior live across big files and big state shapes | iteration cost stays high and UX consistency is harder to maintain |
| Packaging and reliability sit beside the product instead of inside it | operator runbooks and validation exist, but are not part of the product story | reliability work feels secondary instead of core |

## Target Product Shape

The elevated product should feel like this:

- one terminal loop
- one durable work context
- one accepted planning contract
- one clear explanation of current state
- one obvious next action when the system pauses

The operator should be able to answer four questions at any time:

1. What state is the shell in right now?
2. What work is the system trying to do next?
3. Why did automation continue, stop, or pause?
4. What is the fastest safe action to resume progress?

## Chosen Elevation Priorities

### 1. Reduce cognitive overhead before adding major new features

The next iteration should improve state language, flow clarity, and default behavior before investing in broad new capability.

### 2. Make automation more explainable before making it more aggressive

Queue-driven continuation is already the differentiator. The next gain is operator trust, not more hidden autonomy.

### 3. Make planning easier to start and easier to maintain

Planning should feel like a lightweight operating scaffold for a workspace, not like a mini framework the operator has to learn upfront.

### 4. Treat reliability and recovery as product features

Repair, rollback, validation, and operator runbooks should be part of the main value story because they determine whether a power user keeps the tool open all day.

## Capability Pillars

### A. UX Simplification

- establish a smaller top-level state model for the shell
- standardize status language around current state, pause reason, and next action
- make overlay responsibilities non-overlapping
- move deep planner detail behind explicit debug visibility

### B. Planning And Automation Trust

- make queue, proposal, and pause semantics human-readable
- reduce planning authoring friction for the first successful queue-driven loop
- make repair and recovery behavior visible as protections, not strange failures
- clarify what the planner owns versus what the operator owns

### C. Long-Session Continuity

- tighten the relationship between session resume, accepted planning state, and continuation continuity
- make "come back to work" faster than "start over and rediscover state"
- expose next-task intent more clearly in queue and planning surfaces
- build on the shipped workspace lifecycle commands so planning recovery and bootstrap feel lighter during interactive use

### D. Reliability And Distribution

- keep real-terminal validation part of the product operating model
- tighten docs and runbooks for platform-facing confidence
- define product-level acceptance signals beyond unit tests

## Phased Roadmap

### Phase 1: Explain The System Better

Objective:
Make the current feature set easier to understand without changing the product thesis.

Primary work:

- rework shell status language and state taxonomy
- redesign overlay responsibilities around inspect, author, and recover flows
- clarify auto-follow pause reasons and recovery paths
- refresh current-state docs so the shipped product reads coherently

Acceptance signals:

- a new operator can explain the difference between planning invalid, queue idle, and repeated queue head
- queue and planning overlays show next action explicitly
- support and debugging burden shifts from "what does this mean?" to "why did this specific workspace reach this state?"

### Phase 2: Make Planning Feel Lighter And Safer

Objective:
Increase the percentage of workspaces that successfully reach a first queue-driven loop.

Primary work:

- make simple mode the obvious default authoring path
- tighten directions and queue-idle authoring ergonomics
- improve proposal visibility and promotion semantics
- make repair flow more operator-legible and less surprising
- refine the shipped planning lifecycle commands so they stay aligned with simpler authoring and recovery flows

Acceptance signals:

- a power user can create a minimal planning workspace and reach a successful auto-follow continuation without reading multiple docs
- the operator can distinguish executable work, optional follow-up candidates, and blocked work from one inspection surface
- repair archives and recovery paths feel intentional rather than hidden

### Phase 3: Strengthen Long-Run Operation

Objective:
Make the shell feel dependable for all-day usage across resume, review, and repeated planning cycles.

Primary work:

- tighten continuity between resumed sessions and accepted planning state
- improve approval-review visibility and future actionability
- deepen packaging, operator handoff, and validation integration
- reduce architectural hotspots that slow UX iteration

Acceptance signals:

- returning to an existing workspace restores both conversation and work-management context quickly
- platform-specific validation remains routine instead of exceptional
- shell and planning changes ship with less cross-surface regression risk

## Product-Level Success Measures

- Time to first successful queue-driven continuation should drop materially.
- The operator should be able to identify the current pause reason from one surface without cross-referencing docs.
- Planning invalid states should lead to a visible repair or authoring path, not a dead end.
- Session resume should preserve a clear sense of what work is currently active and what work is still pending.

## Explicit Non-Goals

- multi-user planning collaboration
- cloud-hosted control planes
- enterprise workflow orchestration
- broad new frontend modes beyond the inline shell
- shipping `llm-assisted` planning authoring before the manual/simple path is clearly successful

## Related Documents

- [15-ux-flow-rearchitecture.md](15-ux-flow-rearchitecture.md)
- [16-planning-and-automation-evolution.md](16-planning-and-automation-evolution.md)
- [17-structure-and-architecture-debt-map.md](17-structure-and-architecture-debt-map.md)
