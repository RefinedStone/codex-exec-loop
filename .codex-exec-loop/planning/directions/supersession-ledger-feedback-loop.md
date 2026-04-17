# Supersession ledger feedback loop

## Outcome

Keep `task-ledger.json` as the official task source of truth even when several agent sessions are
working in parallel by funneling completion results through hidden planning worker refresh.

## Why this direction exists

Supersession only stays coherent if agent reports do not bypass planning authority. The docs lock in
that agent completion is first reported, then reconciled by hidden planning worker, and only then
becomes official queue state for the next assignment.

## Long-horizon plan

- serialize agent-completion refresh through hidden planning worker
- feed agent id, task id, commit, validation, and summary context into ledger refresh
- distinguish reported completion from official completion in every surface
- prevent unchanged queue-head repetition after agent milestones

## Near-term bias

- make the completion payload and serialized refresh contract explicit before adding more agents
- keep official assignment blocked until ledger refresh finishes
- treat planning invalidity as a supersession blocker, not a silent background issue
- prefer explicit reported-versus-official state over inferred completion

## Relevant inputs

- `docs/supersession/01-product-model.md`
- `docs/supersession/03-agent-session-lifecycle.md`
- `docs/supersession/04-task-ledger-feedback-loop.md`
- `docs/supersession/10-implementation-slices.md`
- `src/application/service/planning_worker_orchestration_service.rs`
- `src/application/service/planning_runtime_facade_service.rs`
- `src/application/service/planning_runtime_policy_service.rs`

## Task derivation guidance

- derive slices around one officiality boundary at a time
- keep hidden planning worker reuse explicit in naming and tests
- prefer serialization and provenance fixes before deriving richer follow-up work

## Avoid

- direct ledger writes from agent sessions
- treating raw agent text as ready work without planning refresh
