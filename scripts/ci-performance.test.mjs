import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import { compareSamples, createCargoObserver, distribution, groupLaneSamples, measure, summarizeWorkflow } from "./ci-performance.mjs";

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
  assert.deepEqual(distribution([5, 1, 4, 2]), { samples: 4, median: 3, p90: 5, minimum: 1, maximum: 5 });
  assert.equal(distribution([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]).p90, 10);
  assert.deepEqual(distribution([null, undefined]), { samples: 0, median: null, p90: null, minimum: null, maximum: null });
});
test("lane samples split Cargo build and test execution by restored target cache and skip failed commands", () => {
  const record = (label, seconds, finished, testResults, exitCode = 0) => ({ label, seconds, exitCode, cargo: { finished, testResults } });
  const restored = (hit) => [{ phase: "restored", caches: { target: { hit } } }, { phase: "complete", caches: { target: { hit: "exact" } } }];
  const groups = groupLaneSamples([
    { runId: "1", lane: "rust", snapshots: restored("fallback"), records: [
      record("tests", 300, ["Finished `test` profile [unoptimized + debuginfo] target(s) in 2m 06s"],
        ["test result: ok. 9 passed; 0 failed; finished in 150.50s", "test result: ok. 1 passed; 0 failed; finished in 0.50s"]),
      record("tests", 9, [], [], 101)] },
    { runId: "2", lane: "rust", snapshots: restored("miss"), records: [record("tests", 400, ["Finished `test` profile in 4.5s"], [])] },
    { runId: "3", lane: "node", snapshots: [], records: [record("node", 30, [], [])] },
  ]);
  assert.deepEqual(groups.map(({ lane, targetCache, runs }) => [lane, targetCache, runs]),
    [["rust", "fallback", ["1"]], ["rust", "miss", ["2"]], ["node", "unrecorded", ["3"]]]);
  assert.equal(groups[0].cargoBuildSeconds.median, 126);
  assert.equal(groups[0].testExecutionSeconds.median, 151);
  assert.equal(groups[0].wallSeconds.samples, 1);
  assert.equal(groups[1].cargoBuildSeconds.median, 4.5);
});
test("comparison separates cohorts, cache scenarios and environment and rejects failed samples", () => {
  const environment = { runner: "macos-15", architecture: "arm64", toolchain: "1.95.0", features: "all", profile: "test", target: "aarch64-apple-darwin" };
  const sample = { cohort: "baseline", environment, scenario: "cold", gateSeconds: 20, workflowSeconds: 21,
    runnerSeconds: 50, incompleteJobs: 0, missingDurationJobs: 0, status: "completed", conclusion: "success" };
  const groups = compareSamples([sample, { ...sample, gateSeconds: 30 }, { ...sample, scenario: "cache-reuse" },
    { ...sample, environment: { ...environment, features: "default" } },
    { ...sample, cohort: "candidate", gateSeconds: 10 }]);
  assert.equal(groups.length, 4);
  assert.equal(groups[0].gateSeconds.samples, 2);
  assert.equal(groups[0].gateSeconds.median, 25);
  assert.equal(groups[0].cohort, "baseline");
  assert.equal(groups[3].cohort, "candidate");
  assert.equal(groups[3].gateSeconds.median, 10);
  assert.throws(() => compareSamples([{ ...sample, cohort: undefined }]), /cohort/);
  assert.throws(() => compareSamples([{ ...sample, incompleteJobs: 1 }]), /successful/);
  assert.throws(() => compareSamples([{ ...sample, missingDurationJobs: 1 }]), /successful/);
  assert.throws(() => compareSamples([{ ...sample, runnerSeconds: -1 }]), /successful/);
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

test("incomplete and missing job durations cannot become complete performance samples", () => {
  const run = { run_started_at: "2026-11-24T00:00:00Z", status: "completed", conclusion: "success" };
  const gate = { name: "CI Gate", status: "completed", conclusion: "success",
    started_at: "2026-11-24T00:00:10Z", completed_at: "2026-11-24T00:00:20Z" };
  const metadata = { cohort: "baseline", scenario: "cold",
    environment: { runner: "macos-15", architecture: "arm64", toolchain: "1.95.0",
      features: "all", profile: "test", target: "aarch64-apple-darwin" } };
  for (const worker of [
    { name: "Rust", status: "in_progress" },
    { name: "Rust", status: "completed", conclusion: "success" },
    { name: "Rust", status: "completed", conclusion: "success",
      started_at: "2026-11-24T00:00:10Z", completed_at: "2026-11-24T00:00:00Z" },
  ]) {
    const summary = summarizeWorkflow(run, [gate, worker]);
    assert.equal(summary.runnerSeconds, null);
    assert.equal(summary.workflowSeconds, null);
    assert.equal(summary.incompleteJobs + summary.missingDurationJobs, 1);
    assert.throws(() => compareSamples([{ ...summary, ...metadata }]), /successful/);
  }
  const skipped = summarizeWorkflow(run, [gate, { name: "Optional", status: "completed", conclusion: "skipped" }]);
  assert.equal(skipped.runnerSeconds, 10);
  assert.equal(skipped.missingDurationJobs, 0);
  assert.equal(compareSamples([{ ...skipped, ...metadata }]).length, 1);
  const skippedGate = summarizeWorkflow(run, [{ ...gate, conclusion: "skipped" }]);
  assert.equal(skippedGate.gateSeconds, null);
});

test("measurement cancellation cannot become success when the child handles a signal and exits zero", async () => {
  const directory = mkdtempSync(join(tmpdir(), "akra-ci-cancel-"));
  try {
    for (const [signal, expectedExit] of [["SIGTERM", 143], ["SIGINT", 130]]) {
      const signalSource = new EventEmitter();
      const output = join(directory, signal);
      const code = await measure("cancelled-fixture", "fake-child", [], {
        directory: output,
        signalSource,
        spawnCommand() {
          const child = new EventEmitter();
          child.stdout = new PassThrough();
          child.stderr = new PassThrough();
          child.killed = false;
          child.kill = (requested) => {
            assert.equal(requested, signal);
            child.killed = true;
            setImmediate(() => child.emit("close", 0, null));
            return true;
          };
          setImmediate(() => signalSource.emit(signal));
          return child;
        },
      });
      assert.equal(code, expectedExit);
      const record = JSON.parse(readFileSync(join(output, "steps.jsonl"), "utf8"));
      assert.equal(record.exitCode, expectedExit);
      assert.equal(record.childExitCode, 0);
      assert.equal(record.requestedSignal, signal);
      assert.equal(signalSource.listenerCount("SIGTERM"), 0);
      assert.equal(signalSource.listenerCount("SIGINT"), 0);
    }
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
