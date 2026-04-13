# Hexagonal Runtime Architecture

This document keeps the stable ownership boundaries for the native client.

## Layer Ownership

- `adapter/inbound/tui`: terminal input, shell state mutation, presentation assembly, and render dispatch
- `application/service`: startup, session, conversation, follow-up, and planning orchestration
- `application/port/outbound`: outbound boundaries owned by the application layer
- `adapter/outbound`: concrete app-server, filesystem, and GitHub-polling adapters
- `domain`: UI-neutral models for diagnostics, sessions, conversations, templates, reviews, and planning

Dependency flow stays inward: `adapter -> application -> domain`.

## Runtime Boundary

- startup checks, session loading, snapshot loading, and turn streaming all enter through the app-server adapter
- the shared runtime connection is reused where possible and held for the lifetime of a streaming turn
- request-style work can fall back to an isolated runtime only while the shared path is busy
- protocol objects are mapped into domain-level conversation events before the TUI sees them
- reconnect and reset warnings come back through application services and surface as shell notices

## Planning Boundary

- planning bootstrap, validation, prompt-fragment assembly, queue projection, and reconciliation live in `application/service`
- filesystem reads and writes for planning files stay behind `PlanningWorkspacePort`
- hidden planner session execution stays behind `PlanningWorkerPort`; main conversation streaming stays behind `CodexAppServerPort`
- the queue snapshot is derived state, not an operator-authored source of truth
- repair prompts and protected-file restoration stay outside the TUI layer

## Current TUI Posture

- the product boundary is already hexagonal
- several TUI flows use reducer/effect seams, but rendering still reads `NativeTuiApp` directly in places
- that internal cleanup is a maintenance concern inside the inbound adapter, not a reason to collapse application or domain boundaries
