# Queue Idle Review Prompt

Use this prompt only when the executable queue is empty.

- Treat `docs/design/01-current-product-state.md`, `docs/design/04-hexagonal-runtime-architecture.md`, `docs/design/06-planning-runtime-and-draft-editor.md`, `docs/plan/20-context-first-architecture-and-doc-coherence.md`, `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`, and the current direction detail docs as the roadmap context for this workspace.
- Treat `docs/plan/17-structure-and-architecture-debt-map.md` as the supporting hotspot map for context-first refactor ordering.
- Treat `docs/supersession/README.md` and the rest of `docs/supersession/` as historical context only unless a live validation or operator-facing gap explicitly points back there.
- Treat `directions.toml` as the durable strategy map. Long-lived objectives belong there; immediate execution slices belong in `task-ledger.json`.
- Keep the queue narrow. When the next slice is concrete, derive at most one `ready` or `in_progress` task.
- Prefer follow-up work that reduces context fan-in, normalizes operator vocabulary, clarifies Codex-only coupling, or advances the terminal-agent research matrix with a concrete artifact.
- Prefer capability-boundary notes and small, reviewable audits before broad provider abstractions or speculative adapter scaffolding.
- Do not create product-implementation tasks for Claude or other external terminal agents until the research path names a primary route and fallback route explicitly.
- Keep alternate or broader follow-up work as `proposed`.
- Do not recreate completed supersession directions or revive already-shipped authority-store work as faux roadmap churn.
- Do not create queue work that only edits planning files unless planning maintenance itself is the explicit goal.
- If a refactor is justified, tie it to one operator-visible benefit in `direction_relation_note` and `description`.
- If no justified implementation slice exists, keep the queue empty and briefly explain why.
