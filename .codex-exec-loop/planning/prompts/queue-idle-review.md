# Queue Idle Review Prompt

Use this prompt only when the executable queue is empty.

- Treat `docs/design/01-current-product-state.md`, `docs/design/04-hexagonal-runtime-architecture.md`, `docs/design/06-planning-runtime-and-draft-editor.md`, `docs/plan/20-context-first-architecture-and-doc-coherence.md`, `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`, and the current direction detail docs as the roadmap context for this workspace.
- Treat `docs/plan/22-terminal-agent-transport-and-attachment-matrix.md`, `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`, and `docs/plan/24-terminal-agent-bridge-experiment-matrix.md` as the supporting research set when terminal-agent transport, headless runner attachment truth, managed launch, or proxy and vibeProxy-style questions are in scope.
- Treat `docs/plan/17-structure-and-architecture-debt-map.md` as the supporting hotspot map for context-first refactor ordering.
- Treat `docs/supersession/current-contract.md` and the rest of `docs/supersession/` as the current shipped contract hub, not as historical-only context.
- Treat DB direction authority as the durable strategy map. Long-lived objectives belong there; immediate execution slices belong in DB task authority.
- Keep the queue narrow. When the next slice is concrete, derive at most one `ready` or `in_progress` task.
- Prefer follow-up work that reduces context fan-in, normalizes operator vocabulary, clarifies Codex-only coupling, or advances the planning-worker-only Claude headless runner lane with one concrete artifact.
- Prefer capability-boundary notes and small, reviewable audits before broad provider abstractions or speculative adapter scaffolding.
- Do not create queue work for main interactive Claude or broader external terminal-agent runtime replacement until the planning-worker-only headless runner baseline is stable.
- Keep alternate or broader follow-up work as `proposed`.
- Do not recreate completed supersession directions or revive already-shipped authority-store work as faux roadmap churn.
- Do not create queue work that only edits planning files unless planning maintenance itself is the explicit goal.
- If a refactor is justified, tie it to one operator-visible benefit in `direction_relation_note` and `description`.
- If no justified implementation slice exists, keep the queue empty and briefly explain why.
