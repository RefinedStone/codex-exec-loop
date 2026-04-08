# Event-Driven UI PoC

This PoC is a design probe, not the production baseline.

## What It Proves
- inbound shell input and async completions can be normalized into one event stream
- a pure reducer plus effect model can fit inside the inbound adapter without breaking hexagonal ownership
- application services and outbound ports do not need to change just to test a reducer-style shell

## Where It Lives
- `src/adapter/inbound/tui/event_driven_poc.rs`

## When To Reuse It
Use this note only if phase 2 revisits reducer-driven shell state. Do not treat it as an implementation commitment on its own.
