# Execution Order

This file now captures the current delivery posture rather than a standing step-by-step sequence.

## Current Delivery Posture
1. Preserve the live shell baseline and avoid regressing shipped runtime or automation behavior.
2. Let future feature docs own detailed sequencing once a concrete workstream exists.
3. Prefer changes that keep runtime, shell ergonomics, and automation behavior explainable from the existing docs.
4. Refresh regression coverage whenever a change alters runtime, shell flow, or auto follow-up behavior.
5. When multiple branches move in parallel, split the work with `04-worktree-branch-rules.md` and `11-parallel-worktree-plan.md` before opening new worktrees.
6. When a PR changes terminal restore or platform-facing shell behavior, use `12-platform-validation-matrix.md` as the required manual validation checklist.

## Handoff Rule
When a concrete feature doc exists under `plan/`, that feature doc should carry the detailed plan for that workstream and can be as detailed as the change requires. This file should remain a short statement of the current project posture instead of accumulating stale ordering detail.
