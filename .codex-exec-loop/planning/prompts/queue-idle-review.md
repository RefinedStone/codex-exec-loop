# Queue Idle Review Prompt

Use this prompt only when the executable queue is empty. The planning worker is a post-turn evaluator, not a TODO extractor for the main session.

- Treat `docs/design/01-current-product-state.md`, `docs/design/04-hexagonal-runtime-architecture.md`, `docs/design/06-planning-runtime-and-draft-editor.md`, `docs/plan/20-context-first-architecture-and-doc-coherence.md`, `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`, and the current direction detail docs as the roadmap context for this workspace.
- Treat `docs/plan/22-terminal-agent-transport-and-attachment-matrix.md`, `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`, and `docs/plan/24-terminal-agent-bridge-experiment-matrix.md` as the supporting research set when terminal-agent transport, headless runner attachment truth, managed launch, or proxy and vibeProxy-style questions are in scope.
- Treat `docs/plan/17-structure-and-architecture-debt-map.md` as the supporting hotspot map for context-first refactor ordering.
- Treat `docs/supersession/current-contract.md` and the rest of `docs/supersession/` as the current shipped contract hub, not as historical-only context.
- Treat DB direction authority as the durable strategy map. Long-lived objectives belong there; immediate execution slices belong in DB task authority.
- Treat `main-session-latest-reply` as evidence only. It is not completion authority, and a completion claim is not enough unless it satisfies the current DB direction success criteria and task/queue state.
- Compare the latest operator request and main-session result against DB direction goals, success criteria, detail docs, accepted task authority, and DB queue projection.
- Create or update a task when direction criteria remain unmet, validation is missing, or a concrete next execution slice is clear, even if the main reply has no explicit TODO list.
- Ignore older prompt or direction wording that uses `directions.toml`, `task-ledger`, or "latest answer clearly implies" as the completion test; accepted DB authority and evaluator judgment win.
- If the latest operator request asked for nontrivial code, DB, runtime, or planning behavior changes and accepted DB task authority is empty or has no matching completed task, do not leave the queue empty solely because the main reply reports completion, tests, merge, or validation. Create one narrow independent review, verification, or hardening task unless DB authority itself proves no work remains.
- Keep the queue narrow. When the next slice is concrete, derive at most one `ready` or `in_progress` task.
- Prefer follow-up work that reduces context fan-in, normalizes operator vocabulary, clarifies Codex-only coupling, or advances the planning-worker-only Claude headless runner lane with one concrete artifact.
- Prefer capability-boundary notes and small, reviewable audits before broad provider abstractions or speculative adapter scaffolding.
- Do not create queue work for main interactive Claude or broader external terminal-agent runtime replacement until the planning-worker-only headless runner baseline is stable.
- Keep alternate or broader follow-up work as `proposed`.
- Do not recreate completed supersession directions or revive already-shipped authority-store work as faux roadmap churn.
- Do not create queue work that only edits planning files unless planning maintenance itself is the explicit goal.
- If a refactor is justified, tie it to one operator-visible benefit in `direction_relation_note` and `description`.
- If no justified implementation slice exists, keep the queue empty and briefly explain why.
