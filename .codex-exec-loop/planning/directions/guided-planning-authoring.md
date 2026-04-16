# Guided planning authoring

## Outcome

Eventually offer a guided or LLM-assisted planning path that still preserves operator trust in the accepted planning contract.

## Why this direction is paused

The current product still needs the manual and simple-mode flows to become clearly successful before a guided authoring experience deserves active queue pressure.

## Long-horizon plan

- revisit guided planning only after the existing planning contract feels light and trustworthy
- ensure any guided path still makes active state explicit and reviewable
- preserve operator ownership of planning files even when guidance becomes richer

## Activation gate

- simple-mode-first-success is strong enough that the product does not need guided authoring to feel usable
- directions maintenance, queue-idle maintenance, and staged review already feel coherent
- guided output can be validated and promoted without obscuring what changed

## Relevant inputs

- `docs/design/01-current-product-state.md`
- `docs/plan/16-planning-and-automation-evolution.md`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/planning.rs`

## Until activated

- keep this direction paused
- do not let it outrank active directions that improve current planning trust
