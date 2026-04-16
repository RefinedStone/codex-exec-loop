# Architecture boundaries for operator UX

## Outcome

Create smaller seams that let status, queue, automation, and planning-entry work ship without repeatedly reopening the same hotspots.

## Why this direction exists

Several shell and planning hotspots still mix projection, runtime state, and layout work. That makes otherwise small operator-facing changes expensive and harder to review safely.

## Long-horizon plan

- separate status projection from layout and rendering concerns
- separate conversation runtime concerns from automation-runtime concerns where it reduces coupling
- isolate planning-entry flow from the broader planning runtime surface
- organize tests around operator-facing journeys instead of only current file ownership

## Near-term bias

- extract only the seams needed for the first four active directions
- keep refactors tied to one visible operator benefit at a time
- reduce hotspot size without inventing a broad architectural rewrite

## Relevant inputs

- `docs/plan/17-structure-and-architecture-debt-map.md`
- `src/adapter/inbound/tui/app/conversation_runtime.rs`
- `src/adapter/inbound/tui/app/shell_presentation.rs`
- `src/adapter/inbound/tui/app/planning/controller.rs`
- `src/application/service/session_service.rs`

## Task derivation guidance

- every refactor slice should name the operator-visible change it unlocks
- prefer extracting shared projections and reducers before splitting rendering files further
- keep tests close to the operator flow affected by the extraction

## Avoid

- abstract cleanups with no visible product unlock
- multi-hotspot rewrites that block ongoing UX work for too long
