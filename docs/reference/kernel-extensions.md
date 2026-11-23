# Kernel extension host

The first trusted extension host is implemented for repository and local OS user scopes. It does not install or execute external extension packages. The existing `akra.tech-blog` route remains a compatibility adapter; new packages register their manifest and Rust handler in `src/composition/kernel_extensions.rs`.

## Authority and attachment

A normal `akra/kernel/attach` keeps the repository connection. `akra/kernel/extensions/attach` with `{"scope":"local_user","client_name":"desktop"}` adds a second attachment without changing that repository. The OS-private Gateway and the process-locked private state directory determine user and host identity. Client-supplied host IDs, OS users, paths and thread IDs are not accepted as authority. The response contains the server's opaque `kernel` ID and a new host `generation`.

Both scopes use `extensions/list`, `extensions/call` and `extensions/changed`. Local detach releases only that client's subscription; closing a panel or disconnecting a client does not cancel a job. A restarted host receives a new generation, restores desired activation, and marks unfinished jobs `interrupted`; it does not automatically rerun them.

## Registration and schemas

Each static package declares ID, package/API versions, scope, optional panel, permissions and operations. Each operation owns standalone input, output and optional progress JSON Schema documents. Local references stay within that document. Remote references are rejected, so registration performs no remote schema fetch. All declared and implemented actions must match before a package is admitted. A broken package is reported in the catalog with registration errors while valid packages remain available; a duplicate never replaces a previously registered package.

The generic host validates request size and schema, API compatibility, granted permissions and current activation before admission and dispatch. It validates returned values against the registered output schema. The catalog also publishes the same schemas for `configure`, `jobs`, `job` and `cancel`. Existing built-in kernel command types and their semantics remain unchanged.

## State and background jobs

`configure` requires `enabled` and `expected_revision`. The desired setting, actual lifecycle (`disabled`, `enabled`, `disabling`, `error`), revision and host generation are separate values. State changes are persisted before notifications. Disabling first rejects new work, requests cancellation, waits for worker termination, and invokes the job's cleanup callback before advertising `disabled`. Cleanup failures remain visible and prevent re-enabling until cleanup is retried. Jobs use cooperative cancellation on in-process Rust threads: cancelling does not forcibly interrupt submitted OS I/O. A terminal job state describes worker termination; its separate cleanup field describes physical cleanup. Rust unwind panics become failed jobs. Native aborts and forced process termination require host restart, which restores interrupted status without rerunning the work.

Job operations return a durable receipt immediately. The host bounds active jobs to eight. A request's optional idempotency key is bound to its complete action and input; reusing it for a different request is rejected. Equivalent in-flight work converges using the handler's normalized deduplication key. A new explicit request after completion creates a new job; retrying the old key returns the old receipt.

`jobs {}` returns `{"jobs":[...]}`. `job {"job_id":"..."}` and `cancel {"job_id":"..."}` return one job. States are `queued`, `running`, `cancelling`, `completed`, `cancelled`, `failed` and `interrupted`. Result and progress payloads are hidden while an extension is disabled or disabling.

Completed results are retained for 24 hours, with `result_expires_at_ms` in the receipt. Startup, new admission and a 60-second host sweep invoke cleanup for expired results, then remove result/progress payloads. At 128 stored jobs, admission cleans and retires the oldest terminated job before making room; active jobs are never evicted. Unexpired idempotency receipts remain as tombstones, so an evicted-result retry fails as expired instead of rerunning work. Receipts expire after 24 hours and are bounded to 512.

Events are bounded to 64 buffered changes and progress persistence is throttled to at most ten updates per second. An event includes host generation and a monotonic sequence. Lagged subscribers receive `resync:true`; clients re-read the catalog and jobs. Every catalog entry carries the sequence of that snapshot.

## Headless CLI example

Run the Gateway from the same build, then inspect the registered example without a repository:

```bash
akra extensions list --scope local_user --json
akra extensions call akra.example configure --input '{"enabled":true,"expected_revision":0}' --json
akra extensions call akra.example echo --input '{"text":"hello"}' --json
akra extensions call akra.example count --input '{"steps":10,"delay_ms":10}' --idempotency-key example-run-1 --json
```

Use the revision returned by the current catalog when activation already exists. For repository scope, add `--scope repository --workspace <checkout>`. CLI calls obtain the live catalog, validate the registered input schema, then invoke the same attached host as Desktop. They do not fall back to a second local host when the Gateway is unavailable. The example performs no model calls. CLI JSON retains the Gateway rejection code, message and optional data, including when catalog preflight also rejects. A server success that disagrees with its catalog is reported as a client contract error.

Validation and environment limits are recorded in [kernel extension host validation](../validation/kernel-extension-host.md).

The regression suite covers package isolation, schema namespace collisions, failed persistence, revision conflicts, compatibility/permission rejection, concurrent admission, idempotent retry versus rescan, disable/drain/cleanup, restart without rerun, expiry, stale host generations, output validation, native store locking, simultaneous local/repository attachments and the CLI transport.
