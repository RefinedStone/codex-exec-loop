# Planning Runtime Engagement Repair

This note originally captured the planning/manual prompt drift after the first planning-init rollout.

Most of that repair has already landed on `prerelease`. The purpose of this file is now narrower: keep the document aligned with the implemented runtime behavior and isolate the remaining design decisions that are still open.

Unless noted otherwise, file paths below are relative to the repository root.

## What Landed

- `PlanningRuntimeSnapshot` now separates `Uninitialized`, `Invalid`, `ReadyNoTask`, and `ReadyWithTask`.
- `TurnPromptAssemblyService` owns manual and auto-follow prompt assembly.
- manual submit now goes through the planning-aware prompt assembly path before `PromptOrigin::Manual` is emitted
- planning runtime policy and preview rendering read the typed snapshot instead of relying only on prompt-fragment strings
- journey coverage exists for:
  - `:planning simple -> promote -> manual prompt includes planning context`
  - `builtin next-task -> queue empty -> planning refresh prompt`
  - invalid planning workspace blocks auto follow-up

## Resolved From The Original Note

### 1. Prompt assembly is no longer split only by origin

The original document said manual submit still forwarded the raw composer buffer while only auto follow-up appended planning context.

That is no longer current:

- manual submit builds its prompt through `PlanningRuntimeFacadeService::build_manual_prompt(...)`
- manual and auto-follow prompts both read the same `PlanningRuntimeSnapshot`
- the first manual turn after planning initialization now carries planning context when the workspace is valid

### 2. Planning runtime state is now typed

The original note described planning readiness as a prompt-fragment string with no typed execution contract.

That is no longer current:

- runtime code distinguishes `ReadyNoTask` from `ReadyWithTask`
- queue-head presence is available as typed data
- display state and policy state are both derived from the same snapshot

### 3. The original manual-prompt bug is fixed

The operator-visible mismatch that motivated this document has been narrowed:

- planning-aware manual prompt assembly is already implemented
- the remaining mismatch is about queue-empty auto-follow policy, not about the first manual turn being unmanaged

## Remaining Gaps

### A. Builtin `next-task` queue-empty policy is still a live design choice

Current implementation:

- builtin `next-task` blocks when planning is `Uninitialized`
- invalid planning workspaces still block auto follow-up
- when planning is `ReadyNoTask`, the runtime queues a planning refresh prompt instead of hard-blocking
- if promotable proposals exist, that same refresh path can ask the LLM to promote or prioritize them into executable queue work

This diverges from the stricter rule proposed in the original version of this plan:

- original proposal: builtin `next-task` should require `ReadyWithTask`
- current behavior: builtin `next-task` may act as "refresh queue from the latest answer" when the queue is empty but planning is otherwise valid

What still needs a decision:

- keep the refresh-on-empty behavior and update docs, UI wording, and template semantics to make that the canonical contract
- or restore the stricter gate so builtin `next-task` blocks whenever there is no actionable queue head

### B. Prompt assembly is centralized for manual and auto, but not yet for planning repair

Current implementation:

- manual prompt assembly is centralized
- auto-follow prompt assembly is centralized
- planning repair prompt text is still built in `PlanningReconciliationService`

This is acceptable functionally, but it means the original "single prompt assembly boundary for every origin" goal is only partially complete.

### C. The phase-2 TUI internal refactor is still in progress

The product-level hexagonal boundary remains intact, but the internal TUI cleanup described in `docs/design/04-hexagonal-runtime-architecture.md` is not finished.

Current posture:

- reducer/effect seams exist for conversation runtime, shell chrome, and several UI subflows
- render and presentation code still read `NativeTuiApp` directly in many places
- presentation data is more explicit than before, but the renderer is not yet frontend-neutral in the stronger sense described by the design doc

Treat this as ongoing shell refactor work, not as evidence that the product-level architecture regressed.

## Current Slice

- scope: documentation refresh so the planning-runtime notes match the code now shipped on `prerelease`
- verification:
  - document review against current implementation
  - `cargo test`

## Follow-Up Options

1. Decide the canonical queue-empty contract for builtin `next-task`, then align code and docs the same way.
2. If prompt-assembly symmetry still matters, move planning repair prompt construction behind `TurnPromptAssemblyService` or an adjacent dedicated assembler.
3. Continue the TUI event/presentation split by shrinking direct `NativeTuiApp` reads inside rendering and presentation code.
