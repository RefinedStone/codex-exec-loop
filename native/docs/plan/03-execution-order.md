# Execution Order

This file now captures the current delivery posture rather than a standing step-by-step sequence.

## Current Delivery Posture
1. Preserve the live shell baseline and avoid regressing shipped runtime or automation behavior.
2. Let future feature docs own detailed sequencing once a concrete workstream exists.
3. Prefer changes that keep runtime, shell ergonomics, and automation behavior explainable from the existing docs.
4. Refresh regression coverage whenever a change alters runtime, shell flow, or auto follow-up behavior.

## Handoff Rule
When a future feature doc is added under `plan/`, that feature doc should carry the detailed plan. This file should remain a short statement of the current project posture instead of accumulating stale ordering detail.
