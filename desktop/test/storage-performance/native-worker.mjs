// Explicit local measurement support. No Cargo build, production Gateway, or model request.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { EventEmitter } from "node:events";
import { createInterface } from "node:readline";
import { performance } from "node:perf_hooks";

export const NATIVE_TEST = "adapter::outbound::storage_analysis::benchmark::storage_analysis_performance_worker";
const MAX_LOG_CHARS = 64 * 1024;
const MAX_EVENTS = 10_000;

export function parseNativeEvent(line, runId) {
  const start = line.indexOf("{");
  if (start < 0) return null; // libtest's own output may precede the first JSON on its line.
  let event;
  try { event = JSON.parse(line.slice(start)); }
  catch (error) {
    if (line.includes("storage_performance")) throw new Error("Malformed native performance event", { cause: error });
    return null;
  }
  if (event?.kind !== "storage_performance") return null;
  assert.equal(event.run_id, runId, "native event belongs to the requested run");
  assert.ok(["begin", "progress", "complete"].includes(event.event), "known native performance event");
  assert.ok(Number.isSafeInteger(event.event_unix_us) && event.event_unix_us > 0, "native UNIX microseconds");
  return event;
}

export function scanInterval(events) {
  const begin = events.find(item => item.event === "begin");
  const complete = events.find(item => item.event === "complete");
  assert.ok(begin && complete, "a successful measurement has native begin and complete");
  const metrics = complete.result?.metrics;
  assert.ok(metrics && Number.isFinite(metrics.elapsed_us) && metrics.elapsed_us >= 0, "native scan duration");
  const firstProgress = events.find(item => item.event === "progress");
  const end = metrics.scan_finished_unix_us;
  assert.ok(firstProgress && firstProgress.event_unix_us >= begin.event_unix_us, "native progress follows worker begin");
  assert.ok(Number.isSafeInteger(end) && end >= firstProgress.event_unix_us, "explicit native scan-return timestamp");
  assert.ok(end <= complete.event_unix_us + 1000, "scan cannot finish after its completion report");
  return {
    worker_begin_unix_us: begin.event_unix_us,
    begin_unix_us: firstProgress.event_unix_us,
    end_unix_us: end,
    complete_report_unix_us: complete.event_unix_us,
    start_source: "first_native_progress_callback",
    end_source: "native_scan_return_timestamp",
  };
}

export function inScan(unixUs, interval) {
  return Number.isFinite(unixUs) && unixUs >= interval.begin_unix_us && unixUs <= interval.end_unix_us;
}

export function summarize(values) {
  const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
  if (!sorted.length) return { count: 0, min: null, median: null, p95: null, max: null };
  const middle = Math.floor(sorted.length / 2);
  return {
    count: sorted.length, min: sorted[0],
    median: sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2,
    p95: sorted[Math.max(0, Math.ceil(sorted.length * 0.95) - 1)],
    max: sorted.at(-1),
  };
}

export function deadline(promise, milliseconds, label) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(label + " timed out after " + milliseconds + " ms")), milliseconds);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

export function validateRequest(request) {
  assert.equal(request.mode, "run", "the Desktop probe only runs a prepared fixture");
  assert.ok(["small_many", "large_few"].includes(request.case), "known fixture case");
  assert.ok([1, 2, 4].includes(request.concurrency), "bounded native concurrency");
  assert.ok(typeof request.fixture_root === "string" && request.fixture_root.length > 0, "owned fixture root");
  assert.match(request.run_id, /^[A-Za-z0-9._-]{1,120}$/, "bounded measurement run ID");
  assert.equal(request.cancel_after_first_file, false, "UI measurement uses a completed native scan");
}

export class NativePerformanceRun extends EventEmitter {
  constructor(executable, request, cwd, env) {
    super();
    validateRequest(request);
    this.executable = executable;
    this.request = structuredClone(request);
    this.cwd = cwd;
    this.env = env;
    this.events = [];
    this.stdoutTail = "";
    this.stderrTail = "";
    this.closed = false;
    this.started = false;
    this.failure = null;
    this.done = new Promise((resolve, reject) => { this.resolve = resolve; this.reject = reject; });
    // The caller awaits this after the UI starts; retain early spawn/protocol failures.
    this.done.catch(() => {});
  }

  get scanActive() {
    return this.events.some(event => event.event === "begin")
      && !this.events.some(event => event.event === "complete") && !this.failure && !this.closed;
  }

  fail(error) {
    if (this.failure) return;
    this.failure = error instanceof Error ? error : new Error(String(error));
    this.reject(this.failure);
    this.emit("failure", this.failure);
  }

  start() {
    assert.equal(this.started, false, "one explicit scan starts one native worker");
    this.started = true;
    this.spawnedAt = { monotonic_ms: performance.now(), unix_us: Date.now() * 1000 };
    this.child = spawn(this.executable,
      [NATIVE_TEST, "--ignored", "--exact", "--nocapture", "--test-threads=1"], {
        cwd: this.cwd,
        env: { ...this.env, AKRA_STORAGE_PERFORMANCE_REQUEST: JSON.stringify(this.request) },
        stdio: ["ignore", "pipe", "pipe"],
        windowsHide: true,
      });
    this.child.once("error", error => this.fail(error));
    this.child.stdout.setEncoding("utf8");
    this.child.stderr.setEncoding("utf8");
    this.child.stdout.on("data", text => { this.stdoutTail = (this.stdoutTail + text).slice(-MAX_LOG_CHARS); });
    this.child.stderr.on("data", text => { this.stderrTail = (this.stderrTail + text).slice(-MAX_LOG_CHARS); });
    const lines = createInterface({ input: this.child.stdout, crlfDelay: Infinity });
    lines.on("line", line => {
      try {
        const event = parseNativeEvent(line, this.request.run_id);
        if (!event) return;
        const began = this.events.some(item => item.event === "begin");
        const finished = this.events.some(item => item.event === "complete");
        assert.equal(finished, false, "no native event follows complete");
        if (event.event === "begin") assert.equal(began, false, "one native begin");
        else assert.equal(began, true, "native progress/completion follows begin");
        assert.ok(this.events.length < MAX_EVENTS, "bounded native event stream");
        const received = { ...event, received_monotonic_ms: performance.now(), received_unix_us: Date.now() * 1000 };
        this.events.push(received);
        this.emit("event", received);
      } catch (error) {
        this.fail(error);
        this.child.kill(); // Only the worker created by this probe.
      }
    });
    this.child.once("close", (code, signal) => {
      this.closed = true;
      lines.close();
      if (this.failure) return;
      if (code !== 0 || signal) {
        this.fail(new Error("Native measurement exited " + code + " (" + signal + "): " + this.stderrTail));
        return;
      }
      try {
        const interval = scanInterval(this.events);
        assert.ok(this.events.some(event => event.event === "progress"), "actual native progress was observed");
        const complete = this.events.find(event => event.event === "complete");
        assert.ok(complete.result?.snapshot && complete.result?.root_node, "native result contains the real snapshot");
        this.resolve({
          pid: this.child.pid, exit_code: code, spawned_at: this.spawnedAt, interval,
          events: this.events, result: complete.result, stdout_tail: this.stdoutTail, stderr_tail: this.stderrTail,
        });
      } catch (error) { this.fail(error); }
    });
  }

  async stop() {
    if (!this.child || this.closed) return;
    const closed = new Promise(resolve => this.child.once("close", resolve));
    this.child.kill();
    await deadline(closed, 10_000, "owned native worker cleanup");
  }
}
