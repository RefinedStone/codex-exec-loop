# Context-First Architecture And Doc Coherence

## Goal

Make the next cycle of Akra easier to understand with minimal local context.

One operator flow should usually require:

- one roadmap doc
- one current-truth doc
- one small code cluster

That rule applies to human contributors and LLM-guided edits alike.

## Why This Direction Exists

The product already ships substantial planning, queue, recovery, and session behavior.
The current bottleneck is not missing capability. It is mixed responsibility and wide context fan-in.

Hotspot files, overlapping docs, and drifting vocabulary make safe iteration more expensive than it
needs to be.

## Near-Term Focus

- align README, docs/README, queue-idle guidance, and roadmap docs around one planning vocabulary
- turn the debt map into an explicit refactor order instead of a loose hotspot list
- record Codex-only coupling points before any external terminal-agent discussion widens
- keep docs/design reserved for shipped truth while docs/plan carries future-facing intent

## Acceptance

- the first follow-up audit names the hotspot split order clearly
- vocabulary drift is visible and reducible from one entrypoint set
- refactor slices can cite one operator-visible benefit and one bounded code cluster
- capability boundaries are described before provider abstractions are introduced

## Supporting Docs

- `docs/plan/20-context-first-architecture-and-doc-coherence.md`
- `docs/plan/17-structure-and-architecture-debt-map.md`
- `docs/design/04-hexagonal-runtime-architecture.md`
