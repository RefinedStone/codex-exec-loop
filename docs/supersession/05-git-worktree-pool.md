# Git Worktree Pool

This document defines the target supersession model, not shipped behavior.

## Pool Purpose

The akra pool is the managed set of reusable git worktree slots that agent sessions consume.
It gives the supervisor a finite execution capacity similar to a connection pool.

## Prerequisites

Parallel mode depends on:

- current workspace is a git repository
- repository has a usable `origin` remote
- local git supports `git worktree`
- integration branch `akra` exists or can be created

If these conditions do not fully hold, supersession may still open in degraded mode, but slot
creation and assignment may be blocked.

## Default Layout

| Item | Default |
| --- | --- |
| integration branch | `akra` |
| pool size | `3` |
| slot id format | `slot-1`, `slot-2`, `slot-3` |
| default pool root | sibling directory `../<repo-name>-worktrees/<repo-root-hash>/akra-pool` |
| agent branch prefix | `akra-agent/<slot-id>/` |

`<repo-root-hash>` is a stable short hash derived from the repository's canonical absolute root.
This avoids collisions when multiple clones of the same repository name live under one parent
directory. The pool root may still be overridden by configuration, but the default should stay
predictable and outside the integration checkout.

## Slot State Model

| Slot state | Meaning |
| --- | --- |
| `idle` | worktree exists, checked out from clean `akra`, no active lease |
| `leased` | slot reserved for a task, branch and session launch in progress |
| `running` | agent session is active in the slot |
| `awaiting_cleanup` | agent execution or distributor finished but cleanup is pending |
| `blocked` | slot cannot be reused because reconcile, cleanup, or branch state failed |
| `missing` | expected slot directory does not exist yet |

## Worktree Creation And Reconcile

Parallel mode enable must reconcile the pool:

1. verify or create `akra`
2. ensure pool root exists
3. inspect expected slot paths
4. create missing slots from `akra`
5. verify existing slots point to the expected branch/worktree state
6. mark inconsistent slots `blocked` instead of silently reusing them

If the configured pool root is absent, supervisor computes the default root from the canonical repo
root and repo-root hash before step 2.

## Branch Rules

Each live agent owns one branch shaped as:

`akra-agent/<slot-id>/<task-slug>`

Rules:

- the branch starts from current `akra`
- one agent session owns one branch at a time
- branch reuse across unrelated tasks is forbidden
- cleanup removes the old branch only after integration completes and no local state remains

## Lease Rules

- supervisor may lease only an `idle` slot
- lease records task id, agent id, branch name, worktree path, and lease start time
- slot becomes `running` only after agent session bootstrap succeeds
- failed bootstrap returns the slot to `idle` only if worktree state is still clean

## Cleanliness Contract

A slot is reusable only when all are true:

- checked out to `akra`
- no staged files
- no unstaged tracked changes
- no unexpected untracked files
- no pending merge or rebase metadata

If any check fails, mark the slot `blocked` and require explicit operator recovery.

## Pool Exhaustion

When all slots are non-idle:

- no new agent session is created
- ready tasks stay in official ledger state
- supersession surface reports `pool exhausted`
- operator can wait, cancel, recover a blocked slot, or increase configured pool size later

## Cleanup Sequence

After distributor finishes integration for a slot:

1. inspect git status
2. ensure source branch result is already integrated
3. checkout `akra`
4. reset slot to clean `akra`
5. remove transient files if policy allows
6. verify clean state
7. mark slot `idle`

## Related Docs

- [03-agent-session-lifecycle.md](03-agent-session-lifecycle.md)
- [06-distributor-and-merge-queue.md](06-distributor-and-merge-queue.md)
- [08-capabilities-degraded-mode-and-failures.md](08-capabilities-degraded-mode-and-failures.md)
- [../plan/04-worktree-branch-rules.md](../plan/04-worktree-branch-rules.md)

## Code Impact

Expected entrypoints:

- `src/application/port/outbound`
- `src/adapter/outbound`
- `src/adapter/inbound/cli.rs`
