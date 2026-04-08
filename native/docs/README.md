# Native Docs

This folder is a compact snapshot of the Rust native client after the first development pass on `prerelease`.

The docs now optimize for phase-2 work:

- keep durable context around core logic, runtime boundaries, and automation rules
- compress UI/UX descriptions down to the shipped shape and intent
- avoid PR-sized checklists or overly detailed milestone scripts that would waste LLM context later

## Reading Order
1. Read [design/01-current-product-state.md](design/01-current-product-state.md).
2. Read [design/04-hexagonal-runtime-architecture.md](design/04-hexagonal-runtime-architecture.md).
3. Read [design/03-auto-followup-and-templates.md](design/03-auto-followup-and-templates.md).
4. Use [design/05-known-gaps-and-risk-areas.md](design/05-known-gaps-and-risk-areas.md) and the `plan/` docs for the current planning baseline.

## Compaction Rule
- Core logic docs should keep stable contracts, ownership boundaries, lifecycle notes, and stop-rule behavior.
- UI/UX docs should describe the implemented form at a high level, not preserve every interaction detail.
- `plan/` docs should describe the current planning state and leave detailed future feature design to separate documents when that work actually starts.

## Document Map
- [design/00-main-to-prerelease-delta.md](design/00-main-to-prerelease-delta.md): short baseline delta from `main`
- [design/01-current-product-state.md](design/01-current-product-state.md): shipped capabilities and current product posture
- [design/02-tui-shell-flow.md](design/02-tui-shell-flow.md): compact summary of the current shell shape
- [design/03-auto-followup-and-templates.md](design/03-auto-followup-and-templates.md): core automation behavior
- [design/04-hexagonal-runtime-architecture.md](design/04-hexagonal-runtime-architecture.md): stable architectural boundaries and runtime lifecycle
- [design/05-known-gaps-and-risk-areas.md](design/05-known-gaps-and-risk-areas.md): current risks worth preserving in context
- [design/06-event-driven-ui-poc.md](design/06-event-driven-ui-poc.md): optional design probe, not the current production baseline
- [plan/01-roadmap.md](plan/01-roadmap.md): current planning baseline, not a future milestone script
- [plan/02-todo-backlog.md](plan/02-todo-backlog.md): current open change buckets that still matter across PRs
- [plan/03-execution-order.md](plan/03-execution-order.md): current delivery posture and how future feature docs should take over detail
