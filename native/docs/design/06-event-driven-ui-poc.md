# Event-Driven UI PoC

## Goal
Prove that the native client can move toward a Flutter `Bloc`-style shell without breaking hexagonal boundaries.

## PoC Shape
The PoC lives in:

- `src/adapter/inbound/tui/event_driven_poc.rs`

It separates the inbound shell into four parts:

1. `StreamShellEvent`
   UI input and async completions are normalized into one event stream.
2. `reduce_stream_shell`
   A pure reducer turns `(state, event)` into `(next_state, effects)`.
3. `StreamShellEffect`
   Effect intents describe side effects without running them.
4. `StreamShellEffectHandler`
   The impure runner calls application services and turns results back into events.

## Hexagonal Fit
- The reducer is adapter-local and stays pure.
- The effect handler lives in the inbound adapter, but it only talks to application services.
- Application services still own use-case orchestration.
- Outbound ports and adapters remain unchanged.
- Domain models stay free of UI concerns.

## Why This Matters
The current `app.rs` mixes:

- key handling
- async orchestration
- state mutation
- render preparation

The PoC shows a path toward:

- easier reducer tests
- thinner render code
- a stream-first shell where UI can be redrawn from state only

## Scope
This is intentionally not wired into the production shell yet. It is a design probe to validate the event/effect split before the stream-first CLI pivot lands.
