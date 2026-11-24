import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { compareSamples, createCargoObserver, distribution, summarizeWorkflow } from "./ci-performance.mjs";

test("workflow latency is not the sum of parallel runner time", () => {
  const job = (name, start, end) => ({ name, started_at: start, completed_at: end, status: "completed", conclusion: "success", steps: [] });
  const result = summarizeWorkflow({ run_started_at: "2026-11-24T00:00:00Z", status: "completed" }, [
    job("Rust", "2026-11-24T00:00:00Z", "2026-11-24T00:10:00Z"),
    job("Desktop", "2026-11-24T00:00:00Z", "2026-11-24T00:08:00Z"),
    job("CI Gate", "2026-11-24T00:10:00Z", "2026-11-24T00:10:05Z"),
  ]);
  assert.equal(result.gateSeconds, 605);
  assert.equal(result.runnerSeconds, 1085);
});
test("unfinished workflow and missing gate do not become zero-duration successes", () => {
  const result = summarizeWorkflow({ status: "in_progress" }, [{ name: "Rust", status: "in_progress" }]);
  assert.equal(result.workflowSeconds, null);
  assert.equal(result.gateSeconds, null);
  assert.equal(result.incompleteJobs, 1);
});
test("median and range distinguish missing samples", () => {
  assert.deepEqual(distribution([5, 1, 4, 2]), { samples: 4, median: 3, minimum: 1, maximum: 5 });
  assert.deepEqual(distribution([null, undefined]), { samples: 0, median: null, minimum: null, maximum: null });
});
test("comparison separates cache scenarios and environment and rejects failed samples", () => {
  const environment = { runner: "macos-15", architecture: "arm64", toolchain: "1.95.0", features: "all", profile: "test", target: "aarch64-apple-darwin" };
  const sample = { environment, scenario: "cold", gateSeconds: 20, runnerSeconds: 50, status: "completed", conclusion: "success" };
  const groups = compareSamples([sample, { ...sample, gateSeconds: 30 }, { ...sample, scenario: "cache-reuse" },
    { ...sample, environment: { ...environment, features: "default" } }]);
  assert.equal(groups.length, 3);
  assert.equal(groups[0].gateSeconds.samples, 2);
  assert.equal(groups[0].gateSeconds.median, 25);
  assert.throws(() => compareSamples([{ ...sample, conclusion: "failure" }]), /successful/);
  assert.throws(() => compareSamples([{ ...sample, environment: {} }]), /environment.runner/);
});
test("Cargo output observation survives chunks and color without assuming zero rebuilds is a cache hit", () => {
  const observer = createCargoObserver();
  observer.feed("\u001b[32m   Compi");
  observer.feed("ling demo v1\u001b[0m\n    Finished test in 4s\ntest result: ok. 2 passed");
  observer.finish();
  assert.equal(observer.state.compilingUnits, 1);
  assert.equal(observer.state.finished.length, 1);
  assert.equal(observer.state.testResults.length, 1);
});
test("measurement preserves command failure and records it without executing shell text", () => {
  const directory = mkdtempSync(join(tmpdir(), "akra-ci-metrics-"));
  try {
    const result = spawnSync(process.execPath, ["scripts/ci-performance.mjs", "run", "fixture", "--", process.execPath, "-e",
      "console.error('   Compiling fixture v1'); process.exit(7)"], {
      encoding: "utf8", env: { ...process.env, AKRA_CI_MEASUREMENTS: directory },
    });
    assert.equal(result.status, 7, result.stderr);
    const record = JSON.parse(readFileSync(join(directory, "steps.jsonl"), "utf8"));
    assert.equal(record.exitCode, 7);
    assert.equal(record.cargo.compilingUnits, 1);
    assert.ok(record.seconds >= 0);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
