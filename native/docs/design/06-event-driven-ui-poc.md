# Event-Driven UI PoC

This PoC is not the production baseline, but it is the architectural reference for the final-stage UI split inside the inbound shell.

## What It Proves
- inbound shell input, background runtime messages, and async completions can be normalized into one event stream
- a pure reducer plus explicit effect model can fit inside the inbound adapter without breaking hexagonal ownership
- async startup, session, and conversation work can live behind effect handling instead of leaking through the renderer
- application services and outbound ports do not need to change just to test a reducer-style shell

## Where It Lives
- `src/adapter/inbound/tui/event_driven_poc.rs`

## When To Reuse It
Use this note when phase 2 tightens the shell into an event/effect/reducer/view-model split. Do not treat the PoC file itself as a promise that the shipping implementation will keep the same shape.
