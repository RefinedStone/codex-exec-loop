# Parallel Control-Plane Architecture

This document is the canonical architecture reference for parallel mode,
supersession, and task dispatch.

The short rule is:

```text
TUI intent
  -> application control-plane handle
  -> core runtime projection bridge
  -> domain aggregate decision
  -> durable store / effect runner
  -> projection
  -> TUI rendering
```

The TUI sends intent. The application control-plane serializes mutation and
effect ordering, and the core runtime projection bridge keeps the headless app
runtime aligned with the control-plane read model. The domain decides policy.
The repository/store is the durable source of truth.

## R6 Runtime Decision

The current implementation uses a mutex-serialized synchronous facade around
the parallel control-plane runtime. It is intentionally not a mailbox actor loop.

This keeps the control-plane small while still providing:

- single-writer mutation through `ParallelModeControlPlaneHandle`;
- in-flight effect accounting;
- stale completion dropping;
- wake coalescing;
- durable dispatch-command backpressure;
- one projection source for inbound adapters.

Do not add a second runtime, queue actor, or direct raw `ParallelModeService`
owner inside `core` or TUI unless this decision is explicitly revisited.

## Layer Ownership

| Layer | Allowed | Forbidden |
| --- | --- | --- |
| TUI | Toggle/request/pause/resume intent, selection, overlays, loading markers, rendering snapshots | Capacity math, worker launch decisions, retry decisions, dispatch ordering |
| `core` | Copy application projections into app snapshots, route user intent to the application handle | Owning raw parallel services, creating a second queue, deciding parallel policy |
| Application control-plane | Serialize command handling, coordinate effects, maintain runtime state, expose projections | UI rendering, terminal key handling, durable policy hidden outside domain |
| Domain | Decide eligibility, capacity, stale-event behavior, dispatch state transitions, validation | IO, async runtime, UI, filesystem, database calls |
| Repository/store | Persist authority state, leases, dispatch commands, session records, queue state | Business rules that should be tested in domain |
| Effect runner | Execute concrete side effects requested by the control-plane | Deciding whether the side effect should exist |

## State Ownership

| State | Owner |
| --- | --- |
| Board selection, overlay visibility, prompt lock display | TUI |
| Latest parallel snapshot/status shown to the user | TUI projection cache |
| Runtime wake scheduling, effect IDs, stale completion guards | Application control-plane |
| Task authority, dispatch queue, leases, session records | Durable store |
| Capacity, readiness, worker actionability, supersession validity | Domain |

When in doubt, ask whether the state must survive process restart or affects a
domain invariant. If either is true, it should not live only in TUI state.

## Command Flow

1. The TUI turns a key binding or command into an application intent.
2. The control-plane handle serializes the command.
3. The control-plane loads the necessary authority state and asks domain code
   for the decision.
4. The control-plane records the state transition and emits effects.
5. The effect runner executes side effects and returns completions.
6. Completions re-enter the control-plane and stale completions are discarded.
7. The TUI renders the latest projection without recalculating policy.

## Core Integration

`core` may include parallel projections in `AppSnapshot` so inbound adapters can
render a single app view. That does not make core the owner of parallel
mutation. Parallel commands continue to go through
`ParallelModeControlPlaneHandle`, and the domain/application layers continue to
own policy and ordering.

## Continuation And Epoch Linearization

Post-turn completion can outlive the conversation state that started it. Each request therefore
captures a `PostTurnContinuationPermit`, and parallel completion also captures the current workspace
epoch. Both guards serialize their bounded durable commit boundaries:

- advancing the continuation generation or closing/replacing the epoch linearizes before an old
  commit and rejects it, or waits for a commit that already crossed the boundary;
- the lock order is continuation then epoch, and cancellation never takes the continuation lock;
- worker execution, remote fetch/review, and other long operations run outside these locks; only the
  final authority mutation or distributor enqueue is guarded;
- a result may still be reduced into non-authoritative diagnostics after supersession, but it cannot
  mutate task authority, enqueue delivery, or start a newly unguarded distributor tick.

Official completion refresh adds a durable ordered claim. Every new exact owner token requires both
PID and OS process-start identity; a missing or ambiguous probe fails closed. Heartbeats renew only
the matching owner/order. An explicit successful host apply releases the claim and advances the
executable order; failure, supersession, early return, or RAII drop cancels only the matching claim
without advancing it, so the same head order remains retryable. Nullable PID-only tokens are parsed
only for legacy persisted-claim compatibility and cannot be minted by a new owner.

## Pool Mutation Linearization

The in-process control-plane mutex is not a cross-process pool lock. Every operation that can
allocate, reconcile, reset, recover, integrate through, or clean a worktree slot must also hold the
persistent repository-scoped `PoolMutationLock` below the private pinned pool root. The OS lock has
a bounded wait, records a process-start identity plus CSPRNG nonce, and revalidates the root and lock
object identities before the guarded mutation. It is never replaced with a check-then-create lock
directory or an in-memory-only permit.

Every newly allocated slot also receives 32 bytes from the OS CSPRNG encoded as exactly 64 lowercase
hexadecimal characters. This lease generation is part of the authoritative SQLite lease identity
and is propagated through session detail, lifecycle transitions, worker completion, distributor
enqueue/recovery, and cleanup. Each write compares the expected exact generation. An event from
generation A therefore cannot release, advance, or annotate the same slot after generation B has
been allocated. Generation-less legacy records remain read-compatible only where an explicit
bounded migration/recovery path permits them; a new allocation never mints a legacy identity.

SQLite is authoritative on every supported platform. On Unix, lease, distributor, and session JSON
files are descriptor-anchored, owner-private, link-rejecting, bounded, atomically installed
recovery/debug projections below the pinned pool root. On Windows those mirrors are neither read nor
written until an NT relative-handle implementation exists; reads are cache misses and projection
writes/removals are no-ops. There is no ordinary-path fallback from SQLite authority. The persistent
lock object alone is not managed pool state during first initialization. A missing Unix lease mirror
is created only after the authoritative CAS; an existing mirror must carry the same immutable
generation, and a failed install rolls authority and the mirror back to their exact observed bodies
without overwriting a concurrent replacement generation.

## Frozen Delivery Proof

Parallel delivery does not infer its target from mutable `origin` or `prerelease` names after work
starts. Slot acquisition freezes the configured push remote, credential-redacted canonical HTTPS
URL, GitHub repository and visibility, integration branch, and fetched integration OID. For an
existing pool, the fetched OID must match the prior tracking-ref observation; a missing or advanced
observation requires a stable second run before any slot reset.

The lease target is copied into the distributor record together with the exact source SHA and the
ordered merge-base-to-tip commit range. Isolated fetch/push and GitHub calls use the frozen URL and
credential context, reviewed PR state must refer to the frozen source SHA, and integration occurs in
a generated detached worktree. Configuration, URL, visibility, branch, base, source, review, or
tracking drift blocks before the corresponding remote or durable mutation instead of falling back to
the current checkout state.

## Host Git Execution Boundary

Host-owned Git commands pin a trusted native Git executable, clear inherited Git execution/routing
environment, disable system/global config, hooks, fsmonitor, signing, replacement objects, lazy
fetch, prompts, and external diff use where applicable. That subprocess boundary does not make
repository-local config safe. Immediately before worktree creation, checkout, reset, cherry-pick,
or commit preparation, Akra performs a bounded, NUL-delimited, key-name-only audit of the effective
local plus worktree config, including local includes. Any filter clean/smudge/process/required key,
merge driver, diff command/textconv/external command, interactive diff filter, alternate-ref command,
unverified `core.worktree` redirection, archive command, or merge/diff tool command blocks the
mutation. A normal submodule's verified `core.worktree` is accepted only when it resolves to that
submodule's pinned worktree; nested executable configuration is audited independently. Config values
are not loaded into diagnostics, printed, or executed.

Remote delivery uses a separate frozen network boundary. The configured credential-free GitHub
HTTPS URL, repository identity, source object directory, and token/login proof are copied into an
owner-private isolated Git/config/object context. Repository/global credential helpers,
`insteadOf`, proxy overrides, SSH commands, and routing environment are not imported. Native
`git`/`gh`/`curl`/`bash` paths are pinned outside repository/pool-controlled roots, the reviewed
GitHub helper is embedded and streamed over bounded stdin to `bash --noprofile --norc -s --`, and
token-bearing children receive a minimal environment with validated credential-free proxy and CA
paths. Credentials come only from explicit token variables or trusted `gh auth token`; legacy file
scanning is unsupported. HTTP activity loading rejects redirects and non-HTTPS targets and enforces
one aggregate deadline plus page, item, decoded-byte, and per-response bounds. A hostile or relative
`PATH` blocks instead of selecting an executable from the repository.

## Boundary Gates

`tests/architecture_boundaries.rs` enforces the current boundary: core must not
depend on application DTOs, runtime workers, raw services, or parallel
control-plane internals. New work should keep those gates green. Any proposed
exception is architecture debt and needs an explicit removal path before it is
accepted.

## Review Checklist

- Does the TUI only send intent and render projection?
- Is mutation serialized through `ParallelModeControlPlaneHandle`?
- Are eligibility, capacity, retry, and stale-event decisions in domain code?
- Is durable truth written through the repository/store boundary?
- Are side effects represented as effects and completed back into the
  control-plane?
- Do post-turn authority writes hold both the captured continuation and epoch permit, with network
  work outside the bounded commit section?
- Does every delivery mutation use the lease-frozen target and source proof rather than rereading a
  mutable branch or repository default?
- Does every pool mutation hold and revalidate the repository-scoped OS lock, and does every delayed
  event compare the exact 64-lowercase-hex lease generation before authority mutation?
- Does every host-owned worktree/checkout/reset/cherry-pick/commit mutation re-run the effective
  repository execution-config audit inside the relevant mutation boundary?
- Are token-bearing Git/GitHub processes using pinned native executables and the isolated frozen
  network context, without repository-controlled helpers, interpreters, config, or `PATH` entries?
- Did architecture-boundary tests stay green or move debt downward?

## References

- [`04-hexagonal-runtime-architecture.md`](./04-hexagonal-runtime-architecture.md)
- [`08-parallel-mode-supersession-board.md`](./08-parallel-mode-supersession-board.md)
- [`../supersession/current-contract.md`](../supersession/current-contract.md)
- [`../../tests/architecture_boundaries.rs`](../../tests/architecture_boundaries.rs)
