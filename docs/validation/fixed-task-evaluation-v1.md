# Fixed task evaluation v1 (#2807)

The versioned input and expected metrics live in
`src/application/service/task_delivery_evidence/tests/evaluation-v1.json`.
The test replays each case twice against the existing host verification and
delivery evidence fixture and the production usage accumulator.

Run:

```sh
cargo test -p akra-ade --lib fixed_task_evaluation_replays_independent_metrics -- --nocapture
```

Each case prints JSON with independent metrics:

- Task success: exact synthetic output compared with the objective oracle.
- Valid evidence: current host-passed criteria; waivers counted separately.
- Delivery allowed: actual delivery gate decision, not the task-success oracle.
- Elapsed milliseconds: scripted virtual clock, not real measured latency.
- Tool calls: distinct scripted tool-call identities.
- Observed tokens: deduplicated observations (lower bound), null if missing.
- Usage gaps: missing observation / prefix etc., never treated as zero usage.

The baseline demonstrates correct output with host evidence, correct output
with human waiver and absent usage, incorrect output despite valid host
evidence, and correct output blocked by a stale host receipt. Neither result
quality nor cost is reduced to one score. All inputs are synthetic; no model,
remote account, or external tool is invoked.

# Common runtime conformance (#2808)

Run `cargo test -p akra-ade --lib runtime_conformance -- --nocapture`.
The tests use the same application stream/receipt observer for the real Codex
adapter connected to a local scripted Python peer and an in-memory non-Codex
port. Python 3 must be available on PATH (python on Windows, python3 on Unix).
Six scenarios run through start and resume: retry and duplicate token snapshots,
interruption, failure, unknown terminal status, approval with one-shot
resolution, and cancellation while an approval is pending followed by an
authoritative completion. Missing usage stays unknown. No real Codex process
or model API is called. This is a contract regression, not a benchmark or a
second production adapter.
