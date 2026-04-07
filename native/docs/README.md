# Native Docs

This folder documents the `prerelease` state of the Rust native client and the remaining work needed to move it toward a scrollback-native CLI experience closer to Codex CLI or Claude Code.

## Reading Order
1. Read the current product state first.
2. Read the shell flow and runtime notes.
3. Read the auto follow-up and template design.
4. Use the roadmap and backlog for implementation planning.

## 1. Current Product State
Goal: describe what the latest `prerelease` branch already supports and where it still feels page-based.

References:
- [design/00-main-to-prerelease-delta.md](design/00-main-to-prerelease-delta.md)
- [design/01-current-product-state.md](design/01-current-product-state.md)

## 2. TUI Shell Flow
Goal: describe the current full-screen shell flow, then frame the next pivot toward a single flowing transcript with a fixed bottom composer.

References:
- [design/02-tui-shell-flow.md](design/02-tui-shell-flow.md)

## 3. Auto Follow-Up And Templates
Goal: document builtin follow-up strategies, workspace template loading, stop rules, and the current UI controls around them.

References:
- [design/03-auto-followup-and-templates.md](design/03-auto-followup-and-templates.md)

## 4. Runtime Architecture
Goal: explain the current hexagonal layering, app-server transport shape, and the main architectural gaps that still block a more continuous shell UX.

References:
- [design/04-hexagonal-runtime-architecture.md](design/04-hexagonal-runtime-architecture.md)
- [design/05-known-gaps-and-risk-areas.md](design/05-known-gaps-and-risk-areas.md)

## 5. Roadmap And TODO
Goal: turn the current `prerelease` implementation into a stream-first shell without losing the recent streaming and auto follow-up work.

References:
- [plan/01-roadmap.md](plan/01-roadmap.md)
- [plan/02-todo-backlog.md](plan/02-todo-backlog.md)
- [plan/03-execution-order.md](plan/03-execution-order.md)
