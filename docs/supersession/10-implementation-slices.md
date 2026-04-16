# Implementation Slices

This document defines the target supersession model, not shipped behavior.

## Delivery Strategy

Supersession should land as several reviewable slices. Each slice must leave the product in a
buildable and understandable state even if the full feature is not yet operator-complete.

## Slice Sequence

### Slice 1: Capability Detection And Mode Skeleton

- add `parallel mode` state and command entrypoints
- add capability checks for git, `git worktree`, push, `gh`, and planning readiness
- render placeholder supersession surface in parallel mode

Verification:

- `cargo build`
- `cargo test`
- manual shell check that `:parallel` reports readiness or degraded state

### Slice 2: Supervisor State Model And Empty Panels

- add domain snapshots and supervisor reducer state
- add pool summary, capability summary, and empty agent/queue panels
- keep behavior read-only

Verification:

- rendering tests for supersession panels
- manual terminal check for mode switching

### Slice 3: Git Worktree Pool

- add `GitWorkspacePort`
- implement slot reconcile, lease, release, and cleanup verification
- add pool exhaustion and blocked-slot handling

Verification:

- unit tests for path, branch, and slot state transitions
- manual git sandbox check with missing and dirty slots

### Slice 4: Agent Session Launch And Tracking

- add `AgentSessionPort`
- launch main-grade agent sessions into leased slots
- track lifecycle, running duration, and latest summary

Verification:

- session lifecycle reducer tests
- manual multi-agent launch in a real terminal

### Slice 5: Ledger Refresh Feedback Loop

- capture agent completion report
- serialize hidden planning worker refresh
- separate `reported_complete` from `commit_ready`

Verification:

- planning refresh ordering tests
- repeated queue-head regression tests
- manual check that official assignment waits for ledger refresh

### Slice 6: Distributor Local Merge Queue

- add serial queue processing
- integrate local `akra` update and slot cleanup without GitHub automation first
- surface blocked queue states

Verification:

- queue ordering tests
- conflict and cleanup failure tests
- manual local integration run with two queued items

### Slice 7: GitHub Automation

- add `GithubAutomationPort`
- use `gh` for auth check, PR ensure, merge, and close
- report degraded behavior when `gh` is unavailable

Verification:

- adapter tests for capability parsing
- manual authenticated and unauthenticated checks

### Slice 8: UX Polish And Validation

- refine copy, alerts, compact summary, and selected-agent detail
- document shipped behavior back into `docs/design` when stable
- add real-terminal validation captures

Verification:

- `cargo build`
- `cargo test`
- `cargo clippy --all-targets --all-features -D warnings`
- terminal validation runs covering mode switch, agent activity, ledger refresh, and distributor cleanup

## Parallel Work Guidance

Potential disjoint lanes:

- lane A: supervisor reducer and presentation
- lane B: git worktree and distributor adapters
- lane C: agent session runtime
- lane D: planning feedback loop

Potential hotspots:

- `src/adapter/inbound/tui/app/app_runtime.rs`
- `src/adapter/inbound/tui/app/shell_presentation.rs`
- `src/adapter/outbound/codex_app_server_adapter.rs`
- `src/application/service/planning`

## Exit Criteria For Initial Shippable Version

- parallel mode can enable in a git workspace
- at least one agent can be assigned into a slot
- agent completion becomes official only after ledger refresh
- distributor can integrate one queue item into `akra`
- cleaned slots return to idle
- degraded capability states are visible and actionable

## Related Docs

- [05-git-worktree-pool.md](05-git-worktree-pool.md)
- [06-distributor-and-merge-queue.md](06-distributor-and-merge-queue.md)
- [09-architecture-boundaries.md](09-architecture-boundaries.md)
- [../plan/11-parallel-worktree-plan.md](../plan/11-parallel-worktree-plan.md)

## Code Impact

Expected entrypoints:

- `src/adapter/inbound/tui/app`
- `src/application/service`
- `src/adapter/outbound`
