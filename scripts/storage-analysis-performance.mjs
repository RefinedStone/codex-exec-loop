#!/usr/bin/env node
// Opt-in, fixed-fixture measurements. No volume-wide scan, cache eviction or cleanup.
import assert from "node:assert/strict";
import { spawnSync, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { performance } from "node:perf_hooks";

const scanWorkerTest = "adapter::outbound::storage_analysis::benchmark::storage_analysis_performance_worker";
const thresholdWorkerTest = "adapter::outbound::storage_analysis::benchmark::storage_analysis_threshold_performance_worker";
const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const defaultCases = ["small_many", "large_few"];
const order = [[1, 4, 2], [4, 2, 1], [2, 1, 4]];

function options(arguments_) {
  const result = { rounds: 3, reuse: false, threshold: false };
  const names = new Map([
    ["--native-executable", "executable"], ["--fixture-root", "fixture"],
    ["--output", "output"], ["--device-label", "device"], ["--metadata", "metadata"],
    ["--rounds", "rounds"],
  ]);
  for (let i = 0; i < arguments_.length; i += 1) {
    const argument = arguments_[i];
    if (argument === "--reuse-fixture") result.reuse = true;
    else if (argument === "--threshold-only") { result.threshold = true; result.reuse = true; }
    else if (argument === "--help") {
      console.log("node scripts/storage-analysis-performance.mjs --native-executable <libtest.exe> --fixture-root <new absolute akra-storage-performance-* directory> --device-label <label> --output <new JSON> [--metadata <JSON>] [--rounds 3..9] [--reuse-fixture] [--threshold-only]");
      process.exit(0);
    } else {
      const name = names.get(argument);
      assert(name && arguments_[i + 1] && !arguments_[i + 1].startsWith("--"), "unknown or missing argument: " + argument);
      result[name] = arguments_[++i];
    }
  }
  for (const name of ["executable", "fixture", "output", "device"]) {
    assert(result[name], "missing --" + name);
  }
  result.rounds = Number(result.rounds);
  assert(Number.isInteger(result.rounds) && result.rounds >= 3 && result.rounds <= 9, "rounds must be 3..9");
  assert(path.isAbsolute(result.fixture) && path.basename(result.fixture).startsWith("akra-storage-performance-"),
    "fixture must be an explicitly named absolute akra-storage-performance-* path");
  result.fixture = path.normalize(result.fixture);
  result.executable = path.resolve(result.executable);
  result.output = path.resolve(result.output);
  assert(fs.statSync(result.executable).isFile(), "native executable is not a file");
  assert(!fs.existsSync(result.output), "output already exists; use a fresh output path");
  return result;
}

function git(arguments_) {
  return execFileSync("git", arguments_, { cwd: repository, encoding: "utf8" }).trim();
}

async function fileHash(file) {
  const hash = createHash("sha256");
  for await (const chunk of fs.createReadStream(file)) hash.update(chunk);
  return hash.digest("hex");
}

async function provenance(executable) {
  const changed = new Set([
    ...git(["diff", "--name-only", "HEAD"]).split(/\r?\n/),
    ...git(["ls-files", "--others", "--exclude-standard"]).split(/\r?\n/),
  ].filter(Boolean));
  const sources = {};
  for (const file of [...changed].sort()) {
    // Source hashes are captured before any measurements; generated evidence is separate.
    if (!file.endsWith(".rs") && file !== "Cargo.toml" && file !== "Cargo.lock") continue;
    const absolute = path.join(repository, file);
    if (fs.existsSync(absolute) && fs.statSync(absolute).isFile()) sources[file] = await fileHash(absolute);
  }
  return {
    base_commit: git(["rev-parse", "HEAD"]),
    modified_native_source_sha256: sources,
    orchestrator_sha256: await fileHash(fileURLToPath(import.meta.url)),
    executable_name: path.basename(executable),
    executable_sha256: await fileHash(executable),
    worker_test: workerTest,
    build_profile: "Cargo test dev profile; all features; explicit x86_64-pc-windows-msvc target",
  };
}

function runWorker(settings, request) {
  const started = performance.now();
  const processResult = spawnSync(settings.executable, [
    workerTest, "--ignored", "--exact", "--nocapture", "--test-threads=1",
  ], {
    encoding: "utf8", windowsHide: true, timeout: 300_000, maxBuffer: 16 * 1024 * 1024,
    env: { ...process.env, AKRA_STORAGE_PERFORMANCE_REQUEST: JSON.stringify(request) },
  });
  assert(!processResult.error, "native worker failed to execute: " + processResult.error);
  assert.equal(processResult.status, 0,
    "native worker failed:\n" + processResult.stderr + "\n" + processResult.stdout);
  const events = [];
  for (const line of processResult.stdout.split(/\r?\n/)) {
    const start = line.indexOf("{");
    if (start < 0) continue;
    let event;
    try { event = JSON.parse(line.slice(start)); } catch { continue; }
    if (event.kind === "storage_performance") events.push(event);
  }
  assert(events.length > 0, "worker returned no measurement events");
  assert(events.every(event => event.run_id === request.run_id), "worker returned a different run");
  const expected = request.mode.startsWith("prepare") ? "prepared" : "complete";
  assert.equal(events.filter(event => event.event === expected).length, 1, "missing or duplicate terminal measurement");
  return { process_wall_ms: performance.now() - started, events };
}

function quality(result) {
  const root = result.root_node;
  return {
    state: result.snapshot.state,
    files_scanned: result.snapshot.files_scanned,
    directories_scanned: result.snapshot.directories_scanned,
    snapshot_issues: result.snapshot.issues,
    logical_bytes: root.logical_bytes,
    allocated_bytes: root.allocated_bytes,
    known_allocated_bytes: root.known_allocated_bytes,
    root_issues: root.issues,
    children_total_count: result.children.total_count,
    // Node IDs depend on concurrent observation order, so compare names and byte values.
    children: result.children.items.map(node => ({
      name: node.name, logical_bytes: node.logical_bytes,
      allocated_bytes: node.allocated_bytes, known_allocated_bytes: node.known_allocated_bytes,
      state: node.state, issues: node.issues,
    })).sort((a, b) => a.name.localeCompare(b.name, "en")),
    hotspot_rows: result.metrics.threshold_comparisons.map(value => ({
      name: value.name, returned_rows: value.returned_rows, total_count: value.total_count,
      included_bytes: [...value.included_bytes].sort(), has_next_page: value.has_next_page,
    })),
  };
}

function median(values) {
  const sorted = values.toSorted((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}
function range(values) {
  const known = values.filter(value => value !== null && value !== undefined);
  return known.length ? { min: Math.min(...known), median: median(known), max: Math.max(...known) } : null;
}
function summarize(runs) {
  const complete = runs.filter(run => !run.request.cancel_after_first_file);
  const summaries = [];
  for (const caseName of cases) for (const concurrency of [1, 2, 4]) {
    const group = complete.filter(run => run.request.case === caseName && run.request.concurrency === concurrency);
    if (!group.length) continue;
    const metrics = group.map(run => run.result.metrics);
    summaries.push({
      case: caseName, concurrency, samples: group.length,
      run_indices: group.map(run => run.execution_index),
      total_us: range(metrics.map(value => value.elapsed_us)),
      first_progress_us: range(metrics.map(value => value.first_progress_us)),
      first_file_observation_us: range(metrics.map(value => value.first_file_observation_us)),
      first_attempt_total_us: metrics[0].elapsed_us,
      repeat_attempt_total_us: range(metrics.slice(1).map(value => value.elapsed_us)),
      native_peak_working_set_bytes: range(metrics.map(value => value.process_peak_working_set_bytes)),
      children_page_us: range(metrics.map(value => value.children_page_us)),
      default_hotspot_query_us: range(metrics.map(value => value.threshold_comparisons[0].elapsed_us)),
    });
  }
  return summaries;
}

const settings = options(process.argv.slice(2));
const workerTest = settings.threshold ? thresholdWorkerTest : scanWorkerTest;
const cases = settings.threshold ? ["threshold_tree"] : defaultCases;
const prefix = "v3-" + new Date().toISOString().replace(/[-:.TZ]/g, "");
const record = {
  schema_version: 1,
  measured_at: new Date().toISOString(),
  device_label: settings.device,
  device_evidence: settings.metadata ? JSON.parse(fs.readFileSync(settings.metadata, "utf8")) : null,
  environment: {
    platform: process.platform, os_release: os.release(), architecture: process.arch,
    node_version: process.version, logical_cpus: os.availableParallelism(), cpu_model: os.cpus()[0]?.model,
    total_memory_bytes: os.totalmem(),
  },
  provenance: await provenance(settings.executable),
  fixture: {
    location: path.parse(settings.fixture).root,
    directory_name: path.basename(settings.fixture),
    reused_existing_fixture: settings.reuse,
    cold_cache_controlled: false,
    cache_note: "Files were just created or reused; OS cache was neither flushed nor evicted. First means first observed scan in this session, not cold cache.",
    index_placement: "Separate per-run index directory on the same physical volume as the fixture.",
  },
  method: {
    rounds: settings.rounds, rotated_concurrency_orders: settings.threshold ? [[1]] : order,
    dataset_mode: settings.threshold ? "nested native hardlink fixture for threshold comparison" : "sequential and parallel small/large matrix",
    timer_scope: "immediately before begin JSON through adapter.scan return, including progress JSON instrumentation; excludes process startup, fixture creation and completed-result queries",
    first_state_scope: "native progress callback; renderer state is measured by the separate Desktop probe",
    memory_scope: settings.threshold ? "not measured by the separate threshold worker" : "fresh native process peak working set including libtest/runtime/SQLite; excludes Node, Gateway and Electron",
    cancellation_scope: settings.threshold ? "not measured in threshold mode" : "cancel at first file progress; actual dispatch callbacks and cancellation checks instrumented under cfg(test)",
  },
  preparation: null, runs: [], summaries: [], result: "in_progress",
};
fs.mkdirSync(path.dirname(settings.output), { recursive: true });
fs.writeFileSync(settings.output, JSON.stringify(record, null, 2) + "\n", { flag: "wx" });
function checkpoint() {
  record.summaries = summarize(record.runs);
  fs.writeFileSync(settings.output, JSON.stringify(record, null, 2) + "\n");
}
const baselines = new Map();
const seenCases = new Set();
function run(caseName, concurrency, round, cancelled) {
  const index = record.runs.length + 1;
  const request = {
    mode: "run", fixture_root: settings.fixture, case: caseName, concurrency,
    run_id: prefix + "-" + index, cancel_after_first_file: cancelled,
  };
  const observation = runWorker(settings, request);
  const result = observation.events.find(event => event.event === "complete").result;
  const comparable = quality(result);
  if (!cancelled) {
    if (baselines.has(caseName)) assert.deepEqual(comparable, baselines.get(caseName), "sequential/parallel result quality differs");
    else baselines.set(caseName, comparable);
  } else {
    assert.equal(result.snapshot.state, "cancelled");
    assert.equal(result.metrics.cancellation.dispatches_after_cancel, 0, "new directory dispatched after cancellation");
    assert(result.metrics.cancellation.new_dispatch_stop_us <= 1_000_000, "dispatch stop exceeds one second");
  }
  record.runs.push({
    execution_index: index, round, first_scan_of_case_in_session: !seenCases.has(caseName),
    request: { ...request, fixture_root: "<fixture_root>" },
    process_wall_ms: observation.process_wall_ms,
    events: observation.events.filter(event => event.event !== "complete"),
    result, quality: comparable,
  });
  seenCases.add(caseName);
  checkpoint();
  console.log(JSON.stringify({
    device: settings.device, execution_index: index, case: caseName, concurrency, round,
    cancelled, total_ms: result.metrics.elapsed_us / 1000,
    first_progress_ms: result.metrics.first_progress_us / 1000,
  }));
}
try {
  if (settings.threshold) {
    record.preparation = runWorker(settings, {
      mode: "prepare-threshold", fixture_root: settings.fixture, case: "threshold_tree", run_id: prefix + "-prepare",
    });
    checkpoint();
  } else if (!settings.reuse) {
    record.preparation = runWorker(settings, {
      mode: "prepare", fixture_root: settings.fixture, run_id: prefix + "-prepare",
    });
    checkpoint();
  }
  for (let round = 0; round < settings.rounds; round += 1) {
    for (const caseName of cases) {
      for (const concurrency of (settings.threshold ? [1] : order[round % order.length])) run(caseName, concurrency, round + 1, false);
    }
  }
  if (!settings.threshold) {
    for (const caseName of cases) for (const concurrency of [1, 2, 4]) run(caseName, concurrency, null, true);
  }
  record.result = "complete";
  record.finished_at = new Date().toISOString();
  record.quality_equal_across_all_completed_runs = true;
  checkpoint();
} catch (error) {
  record.result = "failed";
  record.failure = String(error);
  checkpoint();
  throw error;
}
