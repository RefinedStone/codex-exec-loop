# Planning entry ergonomics

## Outcome

Make planning entry feel low ceremony while preserving the explicit accepted-planning contract.

## Why this direction exists

The product already ships planning lifecycle commands and simple-mode staging, but the first-run and re-entry experience still foregrounds too much planning machinery. The next improvement is not more capability. It is less friction and clearer promotion language.

## Long-horizon plan

- make simple mode the dominant planning entry path
- explain promotion in terms of what becomes active and what behavior it enables
- keep `akra` and `:` lifecycle commands aligned with the same entry and recovery language
- keep disabled guided paths from dominating the main planning flow

## Near-term bias

- simplify simple-mode review and promotion copy
- reduce the visibility cost of disabled `llm-assisted` affordances
- keep entry and recovery surfaces predictable instead of adding new branches

## Relevant inputs

- `docs/plan/14-product-elevation-blueprint.md`
- `docs/plan/16-planning-and-automation-evolution.md`
- `src/adapter/inbound/tui/app/planning/controller.rs`
- `src/adapter/inbound/tui/app/shell_presentation/overlays/planning.rs`
- `src/adapter/inbound/tui/app/inline_shell_commands.rs`

## Task derivation guidance

- prefer slices that reduce visible ceremony for the first accepted planning loop
- keep promotion explicit even when shortening the surrounding copy
- treat simple mode, `:planning`, and lifecycle commands as one entry story

## Avoid

- reopening guided or LLM-assisted authoring as active work
- making planning promotion implicit or invisible
