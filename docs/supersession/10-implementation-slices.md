# Implementation Slices

This document records the historical supersession slice plan plus the current branch completion status.

## Delivery Strategy

Supersession should land as several reviewable slices. Each slice must leave the product in a
buildable and understandable state even if the full feature is not yet operator-complete.

## Current Branch Status

`origin/prerelease` already ships the first operator-visible supersession loop.

- Slice 1 shipped: readiness gating and `:parallel` mode entry are live.
- Slice 2 shipped: the control-tower board and domain snapshots are live.
- Slice 3 shipped: the `akra` worktree pool reconciles, leases, blocks, and cleans slots.
- Slice 4 shipped in native form: queue-driven handoff launches main-grade agent sessions into leased slots and tracks lifecycle state.
- Slice 5 shipped: completion becomes official only after serialized hidden planning refresh succeeds.
- Slice 6 and Slice 7 shipped together in native form: distributor queue delivery, rebase provenance, GitHub automation, and slot return are live.
- The current branch also implements Slice 9 through Slice 13: authority locator and shadow store, store-backed drafts and promote, active planning mutation and queue claims, runtime projection recovery, and store-primary cutover.
- Post-cutover legacy bootstrap cleanup is also implemented on the current branch.
- The remaining work is validation depth, compact docs alignment, and residual surface polish rather than additional core supersession architecture.

## Shipped Slices

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

Current status:

- partially complete
- shipped behavior has been summarized back into `docs/design/`
- remaining work is validation depth and residual surface polish rather than core supersession enablement

Verification:

- `cargo build`
- `cargo test`
- `cargo clippy --all-targets --all-features -D warnings`
- terminal validation runs covering mode switch, agent activity, ledger refresh, and distributor cleanup

## Branch-Complete Follow-Through

### Slice 9: Authority Locator And Shadow Store

- resolve canonical repo authority location rules
- bootstrap SQLite schema and shadow-store inspection
- mirror tracked planning files into the store before runtime authority moves

Verification:

- parity checks between tracked planning files and mirrored store snapshot
- worktree-level read tests that resolve one canonical authority root

### Slice 10: Store-Backed Drafts And Promote

- move draft storage, validation, and rejection resume data into the authority store
- preserve `draft -> validate -> promote` semantics
- keep active planning unchanged until promote succeeds

Verification:

- draft validation tests
- promote success and rejection-resume tests
- manual authoring flow check in detail mode

### Slice 11: Active Planning Mutation And Queue Claims

- route hidden planning refresh through store-backed active commits
- move official refresh reservation and distributor queue-head claims into the same authority domain
- separate planning revision from runtime event sequence

Verification:

- cross-process claim uniqueness tests
- queue projection commit tests
- repeated queue-head regression tests

### Slice 12: Runtime Projection Migration And Recovery

- move slot, session, and distributor delivery projections into the authority store
- append runtime-domain events and recover from store-backed projections
- recheck Git and GitHub truth before reclassifying in-flight work

Verification:

- recovery tests for in-flight refresh, push, PR ensure, integration, and cleanup
- manual restart checks during blocked and in-flight distributor work

### Slice 13: Store-Primary Cutover

- make store-backed active and draft planning the default runtime authority
- keep tracked planning files as revision-stamped exports
- allow tracked-file import only through explicit operator flow

Verification:

- store-primary end-to-end tests across two worktrees
- export revision labeling checks
- manual review flow for exported planning artifacts

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

Current status:

- achieved on the current branch for both the first operator-visible loop and the repo-shared authority and recovery hardening slices
- remaining work is no longer core supersession enablement

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
