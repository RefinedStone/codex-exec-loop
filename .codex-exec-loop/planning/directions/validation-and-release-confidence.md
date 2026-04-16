# Validation and release confidence

## Outcome

Preserve trust in the shell while the operator-facing product changes quickly.

## Why this direction exists

Operator-facing changes are only durable if validation habits and release handoff stay tight. This direction keeps reliability work tied to the product rather than treated as follow-up maintenance.

## Long-horizon plan

- keep real-terminal validation routine for prompt, queue, planning, and restore changes
- align operator runbooks with evolving shell flows
- make release notes and validation artifacts reflect actual operator risk

## Near-term bias

- pair shell-surface changes with explicit validation notes
- keep runbooks aligned with phase-1 and phase-2 operator flows
- treat packaging and validation docs as part of the delivered experience

## Relevant inputs

- `docs/plan/12-platform-validation-matrix.md`
- `docs/plan/13-native-packaging-and-operator-runbook.md`
- `README.md`

## Task derivation guidance

- derive tasks that directly support upcoming operator-surface changes
- prefer concrete validation or runbook improvements over generic process prose
- keep validation claims grounded in checks that can actually be run

## Avoid

- broad process rewrites disconnected from current product work
- claiming confidence without paired validation evidence
