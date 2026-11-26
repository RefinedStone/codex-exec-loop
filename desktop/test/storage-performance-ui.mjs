// Opt-in native-load measurement, deliberately outside the default e2e/*.mjs inventory.
// Build Desktop and the Rust libtest executable separately, then:
// node --experimental-strip-types desktop/test/storage-performance-ui.mjs --native-executable <exe> --request-json <JSON-or-file> --output <JSON>
import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { basename, dirname, isAbsolute, join, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { fileURLToPath } from "node:url";
import { _electron as electron } from "playwright-core";
import * as fake from "./fake-gateway/fakeGateway.ts";
import { NativePerformanceRun, deadline, inScan, summarize, validateRequest } from "./storage-performance/native-worker.mjs";

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(desktopRoot, "..");
const argumentsMap = new Map();
for (let index = 2; index < process.argv.length; index += 2) {
  const key = process.argv[index], value = process.argv[index + 1];
  if (key === "--help") {
    console.log("Usage: storage-performance-ui.mjs --native-executable <libtest> --request-json <JSON-or-file> --output <report.json> [--timeout-ms 180000]");
    process.exit(0);
  }
  assert.ok(["--native-executable", "--request-json", "--output", "--timeout-ms"].includes(key), "unknown argument: " + key);
  assert.ok(value && !argumentsMap.has(key), "one value for " + key);
  argumentsMap.set(key, value);
}
for (const key of ["--native-executable", "--request-json", "--output"]) assert.ok(argumentsMap.has(key), "required: " + key);
const executable = resolve(argumentsMap.get("--native-executable"));
const requestText = argumentsMap.get("--request-json");
const request = JSON.parse(requestText.trim().startsWith("{") ? requestText : await readFile(resolve(requestText), "utf8"));
validateRequest(request);
assert.ok(isAbsolute(request.fixture_root), "native fixture must be an explicit absolute path");
const outputPath = resolve(argumentsMap.get("--output"));
const timeoutMs = Number(argumentsMap.get("--timeout-ms") ?? 180_000);
assert.ok(Number.isSafeInteger(timeoutMs) && timeoutMs >= 1000 && timeoutMs <= 900_000, "bounded timeout");
await readFile(join(desktopRoot, "out/main/index.js")); // Never silently build or use another checkout.

const ownedRoot = await mkdtemp(join(tmpdir(), "akra-storage-native-ui-"));
const userData = join(ownedRoot, "desktop");
const akraHome = join(ownedRoot, "akra-home");
await mkdir(userData); await mkdir(akraHome);
const isolatedEnv = { ...process.env, AKRA_HOME: akraHome, AKRA_DESKTOP_USER_DATA: userData, AKRA_DESKTOP_HIDDEN: "1" };
delete isolatedEnv.ELECTRON_RUN_AS_NODE;
delete isolatedEnv.AKRA_GATEWAY_BIN;
delete isolatedEnv.AKRA_GATEWAY_ENDPOINT;
const native = new NativePerformanceRun(executable, request, repositoryRoot, isolatedEnv);
const kernel = "local-user-native-performance";
const extension = "akra.storage-analysis";
const generation = "native-performance-fixture";
const volumeId = "fixture-volume";
let app, gateway, view, panel, job = null, nativeResult = null, sequence = 0, scanCalls = 0;
let cleanupPromise;
let resolveScanAccepted;
const scanAccepted = new Promise(resolveAccepted => { resolveScanAccepted = resolveAccepted; });
let interrupted = false;
const calls = [], unsupportedActions = [], memorySamples = [], automationSamples = [], rendererErrors = [];
const memoryErrors = [];
const clone = value => structuredClone(value);
const sleep = milliseconds => new Promise(resolveWait => setTimeout(resolveWait, milliseconds));

function changed() {
  if (gateway && job) gateway.sendExtensionChange(kernel, {
    extension, generation, revision: 1, sequence: ++sequence, job_id: job.id,
  });
}
native.on("event", event => {
  if (!job) throw new Error("native scan began without an explicit Desktop scan");
  if (event.event === "begin") {
    assert.equal(event.volume_id, volumeId, "native fixture volume matches the transport");
    job = { ...job, state: "running", revision: job.revision + 1, sequence: job.sequence + 1 };
  } else if (event.event === "progress") {
    assert.ok(event.progress && typeof event.progress === "object", "actual native progress payload");
    job = { ...job, state: "running", progress: clone(event.progress), revision: job.revision + 1, sequence: job.sequence + 1 };
  } else {
    nativeResult = event.result;
    assert.equal(nativeResult.snapshot.volume_id, volumeId);
    job = { ...job, state: "completed", result: clone(nativeResult.snapshot), cleanup: "complete",
      revision: job.revision + 1, sequence: job.sequence + 1 };
  }
  job.updated_at_ms = Date.now();
  changed();
});
native.on("failure", error => {
  if (job) {
    job = { ...job, state: "failed", result: null, error: "native_measurement_failed: " + error.message,
      revision: job.revision + 1, sequence: job.sequence + 1 };
    changed();
  }
});

function extensionReply(method, params) {
  if (method === "akra/kernel/extensions/attach") return { result: { kernel, scope: "local_user", generation } };
  if (method === "akra/kernel/extensions/detach") return { result: {} };
  if (method === "akra/kernel/extensions/list") return { result: params.kernel === kernel
    ? [{ id: extension, version: 1, scope: "local_user", enabled: true, desired_enabled: true,
      compatible: true, lifecycle: "enabled", generation, revision: 1 }] : [] };
  if (method !== "akra/kernel/extensions/call") return;
  assert.equal(params.kernel, kernel); assert.equal(params.extension, extension);
  const input = params.input ?? {};
  calls.push({ action: params.action, received_unix_us: Date.now() * 1000 });
  if (params.action === "volumes") return { result: { volumes: [{
    id: volumeId, label: "합성 성능 측정 " + request.case, root_path: request.fixture_root, kind: "unknown", supported: true,
  }] } };
  if (params.action === "jobs") return { result: { jobs: job ? [clone(job)] : [] } };
  if (params.action === "job") {
    assert.equal(input.job_id, job?.id);
    return { result: clone(job) };
  }
  if (params.action === "scan") {
    assert.equal(input.volume_id, volumeId);
    assert.equal(++scanCalls, 1, "one UI scan request per measurement");
    job = { id: request.run_id, extension, action: "scan", state: "queued", generation, revision: 1, sequence: 1,
      progress: null, result: null, error: null, cleanup: "pending", created_at_ms: Date.now(), updated_at_ms: Date.now() };
    const accepted = clone(job);
    native.start();
    resolveScanAccepted();
    return { result: accepted };
  }
  // The worker measures native scanning; it does not expose its live hierarchy over this test transport.
  // Returning an explicit unavailable result avoids inventing discovered nodes or byte counts.
  if (params.action === "live_children") {
    unsupportedActions.push(params.action);
    return { error: { code: -32000, message: "performance_probe_live_hierarchy_unavailable" } };
  }
  if (nativeResult) {
    if (params.action === "snapshot" && input.snapshot_id === nativeResult.snapshot.id) return { result: clone(nativeResult.snapshot) };
    if (params.action === "get_node") {
      const node = [nativeResult.root_node, ...(nativeResult.children?.items ?? [])].find(item => item.id === input.node_id);
      if (input.snapshot_id === nativeResult.snapshot.id && node) return { result: clone(node) };
    }
    if (params.action === "list_children" && input.snapshot_id === nativeResult.snapshot.id
      && input.node_id === nativeResult.root_node.id && !input.cursor && !input.name_filter) {
      return { result: clone(nativeResult.children) };
    }
    if (params.action === "find_hotspots" && input.snapshot_id === nativeResult.snapshot.id
      && input.node_id === nativeResult.root_node.id && !input.cursor && !input.thresholds) {
      return { result: clone(nativeResult.hotspots) };
    }
  }
  unsupportedActions.push(params.action);
  return { error: { code: -32602, message: "performance_probe_query_not_exposed: " + params.action } };
}

async function cleanup() {
  if (cleanupPromise) return cleanupPromise;
  cleanupPromise = (async () => {
    const errors = [];
    try { await native.stop(); } catch (error) { errors.push(error.message); }
    if (app) {
      try { await deadline(app.close(), 15_000, "owned Electron cleanup"); }
      catch (error) {
        errors.push(error.message);
        try { app.process().kill(); } catch (killError) { errors.push(killError.message); }
      }
    }
    if (gateway) {
      try { await deadline(gateway.close(), 10_000, "owned test Gateway cleanup"); }
      catch (error) { errors.push(error.message); }
    }
    try { await rm(ownedRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 }); }
    catch (error) { errors.push(error.message); }
    return errors;
  })();
  return cleanupPromise;
}
const onSignal = () => { interrupted = true; void cleanup(); };
process.once("SIGINT", onSignal); process.once("SIGTERM", onSignal);

async function sampleMemory(stage) {
  const started = performance.now();
  try {
    const data = await app.evaluate(({ app: electronApp, BrowserWindow }) => {
      const window = BrowserWindow.getAllWindows()[0];
      const rendererPid = window.webContents.getOSProcessId();
      return {
        window: { visible: window.isVisible(), minimized: window.isMinimized(), focused: window.isFocused() },
        processes: electronApp.getAppMetrics().filter(item => item.type === "Browser" || item.pid === rendererPid)
          .map(item => ({
            pid: item.pid, role: item.type === "Browser" ? "main" : "renderer",
            working_set_kb: item.memory.workingSetSize, peak_working_set_kb: item.memory.peakWorkingSetSize,
            private_kb: item.memory.privateBytes ?? null,
          })),
      };
    });
    memorySamples.push({ stage, unix_us: Date.now() * 1000, sampling_roundtrip_ms: performance.now() - started, ...data });
  } catch (error) { memoryErrors.push({ stage, message: error.message }); }
}

async function inputSample(phase, index) {
  const id = phase + "-" + index, text = "저장 공간 성능 측정 입력 " + id;
  await view.evaluate(({ id: sampleId, phase: samplePhase, text: expected }) => {
    window.__storageNativePerformance.armed = { id: sampleId, phase: samplePhase, expected };
  }, { id, phase, text });
  const started = performance.now(), startedUnix = Date.now() * 1000;
  await view.getByTestId("composer-input").fill(text);
  assert.equal(await view.getByTestId("composer-input").inputValue(), text);
  automationSamples.push({
    id, phase, text_length: text.length, automation_start_unix_us: startedUnix, automation_end_unix_us: Date.now() * 1000,
    automation_to_dom_ms: performance.now() - started,
  });
}

let report;
try {
  gateway = await fake.startFakeGateway(fake.uniqueTestEndpoint(),
    [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
      kernel: { kernel: "unrelated-repository-performance-fixture", queries: {}, onRequest: extensionReply },
    });
  app = await electron.launch({
    executablePath: createRequire(import.meta.url)("electron"),
    args: [desktopRoot, ...(process.env.CI ? ["--no-sandbox"] : [])],
    env: { ...isolatedEnv, AKRA_GATEWAY_ENDPOINT: gateway.endpoint },
  });
  view = await app.firstWindow({ timeout: 30_000 });
  view.setDefaultTimeout(20_000);
  view.on("pageerror", error => rendererErrors.push(error.message));
  await view.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor();
  await view.getByTestId("thread-entry").first().click();
  await view.waitForFunction(() => {
    const input = document.querySelector('[data-testid="composer-input"]');
    return input && !input.disabled;
  });
  await view.getByTestId("storage-toggle").click();
  panel = view.getByRole("region", { name: "저장 공간 분석기", exact: true });
  await panel.getByLabel("분석할 드라이브").selectOption(volumeId);
  await panel.getByRole("button", { name: "분석", exact: true }).waitFor();
  assert.equal(native.started, false, "opening the panel does not start native I/O");

  await view.evaluate(() => {
    const state = { samples: [], status_changes: [], armed: null, click: null, last_status: null,
      initial_visibility: document.visibilityState, clock: { time_origin_ms: performance.timeOrigin, unix_ms: Date.now() } };
    const epochUs = now => Math.round((performance.timeOrigin + now) * 1000);
    window.__storageNativePerformance = state;
    document.addEventListener("input", event => {
      if (!event.target?.matches?.('[data-testid="composer-input"]') || !state.armed) return;
      const armed = state.armed; state.armed = null;
      const start = performance.now();
      const sample = { id: armed.id, phase: armed.phase, input_unix_us: epochUs(start), visibility: document.visibilityState,
        trusted_event: event.isTrusted, dom_value_matches: false, input_to_dom_microtask_ms: null,
        input_to_next_frame_ms: null, input_to_second_frame_ms: null, next_frame_unix_us: null, second_frame_unix_us: null };
      state.samples.push(sample);
      queueMicrotask(() => {
        sample.input_to_dom_microtask_ms = performance.now() - start;
        sample.dom_value_matches = document.querySelector('[data-testid="composer-input"]')?.value === armed.expected;
      });
      requestAnimationFrame(() => {
        const first = performance.now();
        sample.input_to_next_frame_ms = first - start; sample.next_frame_unix_us = epochUs(first);
        requestAnimationFrame(() => {
          const second = performance.now();
          sample.input_to_second_frame_ms = second - start; sample.second_frame_unix_us = epochUs(second);
        });
      });
    }, true);
    document.addEventListener("click", event => {
      const button = event.target?.closest?.("button");
      if (!state.click && button?.closest(".storage-panel") && button.textContent?.trim() === "분석") {
        const now = performance.now(); state.click = { monotonic_ms: now, unix_us: epochUs(now) };
      }
    }, true);
    const status = document.querySelector('.storage-panel [role="status"]');
    if (!status) throw new Error("storage status element is unavailable");
    new MutationObserver(() => {
      const text = status.textContent?.trim() ?? "";
      if (!state.click || !text || text === state.last_status) return;
      state.last_status = text;
      const now = performance.now();
      const entry = { text, unix_us: epochUs(now), click_to_dom_ms: now - state.click.monotonic_ms,
        click_to_next_frame_ms: null, next_frame_unix_us: null };
      if (state.status_changes.length < 256) state.status_changes.push(entry);
      requestAnimationFrame(() => {
        const frame = performance.now();
        entry.click_to_next_frame_ms = frame - state.click.monotonic_ms; entry.next_frame_unix_us = epochUs(frame);
      });
    }).observe(status, { childList: true, characterData: true, subtree: true });
  });
  await sampleMemory("baseline");
  for (let index = 0; index < 5; index++) { await inputSample("baseline", index); await sleep(50); }
  await view.waitForFunction(() => window.__storageNativePerformance.samples.every(sample => sample.second_frame_unix_us !== null),
    null, { timeout: 4500 }).catch(() => {});
  await sampleMemory("before_scan");

  const measurementDeadline = performance.now() + timeoutMs;
  await panel.getByRole("button", { name: "분석", exact: true }).click();
  await deadline(scanAccepted, 5000, "Desktop scan acceptance");
  assert.equal(scanCalls, 1);
  const memoryLoop = (async () => {
    while (!native.closed && !native.failure && !interrupted && performance.now() < measurementDeadline) {
      await sampleMemory("native_worker"); await sleep(250);
    }
  })();
  let index = 0;
  while (!native.closed && !native.failure && !interrupted && performance.now() < measurementDeadline) {
    if (native.scanActive && index < 200) await inputSample("native", index++);
    await sleep(50);
  }
  if (interrupted) throw new Error("measurement interrupted");
  const result = await deadline(native.done, Math.max(1, measurementDeadline - performance.now()), "native measurement");
  await memoryLoop;
  assert.equal(result.result.snapshot.state, "complete", "performance comparison requires a complete native scan");
  await view.waitForFunction(() => window.__storageNativePerformance.samples.every(sample => sample.second_frame_unix_us !== null),
    null, { timeout: 4500 }).catch(() => {});
  await sampleMemory("after_scan");
  await view.getByTestId("composer-input").fill(""); // Never send the synthetic text.
  const observed = await view.evaluate(() => {
    const state = window.__storageNativePerformance;
    return { samples: state.samples, status_changes: state.status_changes, click: state.click,
      initial_visibility: state.initial_visibility, clock: state.clock, final_visibility: document.visibilityState };
  });
  assert.ok(observed.click, "an actual Desktop click was measured");
  const samples = observed.samples.map(sample => ({
    ...sample, ...automationSamples.find(item => item.id === sample.id),
    input_during_native_scan: inScan(sample.input_unix_us, result.interval),
    next_frame_during_native_scan: inScan(sample.next_frame_unix_us, result.interval),
    second_frame_during_native_scan: inScan(sample.second_frame_unix_us, result.interval),
  }));
  assert.equal(samples.length, automationSamples.length, "each measured fill produced one composer input event");
  assert.ok(samples.every(sample => sample.dom_value_matches), "the synthetic input survived the renderer update");
  const overlapping = samples.filter(sample => sample.phase === "native" && sample.input_during_native_scan);
  const statistics = selected => Object.fromEntries([
    "automation_to_dom_ms", "input_to_dom_microtask_ms", "input_to_next_frame_ms", "input_to_second_frame_ms",
  ].map(key => [key, summarize(selected.map(sample => sample[key]))]));
  const modelCalls = gateway.receivedMethods.filter(method => method === "turn/start" || method === "turn/steer");
  assert.equal(modelCalls.length, 0, "the probe must not request a model turn");
  assert.equal(scanCalls, 1);
  const firstState = observed.status_changes[0] ?? null;
  const firstNativeProgressStatus = observed.status_changes.find(status => status.text.includes("확인한 파일")) ?? null;
  const counts = firstNativeProgressStatus?.text.match(/확인한 파일 (\d+)개 · 폴더 (\d+)개/);
  const firstNativeProgress = firstNativeProgressStatus === null ? null : {
    ...firstNativeProgressStatus,
    displayed_files_scanned: counts ? Number(counts[1]) : null,
    displayed_directories_scanned: counts ? Number(counts[2]) : null,
    zero_file_progress: counts ? Number(counts[1]) === 0 : null,
    matching_native_progress_unix_us: counts ? result.events.filter(event => event.event === "progress"
      && event.event_unix_us <= firstNativeProgressStatus.unix_us
      && event.progress.files_scanned === Number(counts[1]) && event.progress.directories_scanned === Number(counts[2]))
      .map(event => event.event_unix_us) : [],
  };
  report = {
    schema_version: 1, measured_at: new Date().toISOString(), status: overlapping.length >= 3 ? "measured" : "insufficient_native_input_samples",
    platform: process.platform, run_id: request.run_id, case: request.case, concurrency: request.concurrency,
    native_executable: basename(executable), fixture_root_name: basename(request.fixture_root),
    versions: await app.evaluate(() => ({ electron: process.versions.electron, chrome: process.versions.chrome, node: process.versions.node })),
    boundary: "Actual NativeStorageAnalysisAdapter fixture scan in its own libtest process plus the real Electron composer and storage status; a synthetic Gateway transports the native progress/result.",
    hidden_electron: true, renderer_visibility: { initial: observed.initial_visibility, final: observed.final_visibility },
    timing_method: {
      input: "Renderer input event to input-value microtask is separate from Playwright fill/inputValue round-trip.",
      frames: "First/second requestAnimationFrame are presentation opportunities, not proof of visible paint. Hidden-window throttling is preserved.",
      native_overlap: "Input and frame UNIX timestamps are compared with the first actual native progress callback through native scan return; worker begin, later report and process exit are not treated as proof of scan overlap.",
      first_status: "Click to first storage status DOM/frame is distinct from first DOM containing an actual native progress payload.",
    },
    exclusions: ["production Gateway scheduling/authentication", "live directory hierarchy rendering", "physical key-to-visible-pixel latency",
      "whole-volume scanning", "GPU/utility/test-transport memory", "model turns"],
    request_counts: { native_workers: 1, scan: scanCalls, model_turns: modelCalls.length },
    extension_request_counts: Object.fromEntries([...new Set(calls.map(call => call.action))]
      .map(action => [action, calls.filter(call => call.action === action).length])),
    native_interval: result.interval, native_metrics: result.result.metrics,
    native_event_count: result.events.length, native_events: result.events,
    native_exit_code: result.exit_code,
    renderer_clock: observed.clock, click: observed.click, first_status: firstState, first_native_progress_status: firstNativeProgress,
    targets: { first_status_dom_under_1s: firstState === null ? null : firstState.click_to_dom_ms < 1000,
      visible_paint_under_1s: null, sufficient_native_input_samples: overlapping.length >= 3 },
    sample_counts: { baseline: samples.filter(sample => sample.phase === "baseline").length, native_attempted: index,
      native_input_overlap: overlapping.length, native_second_frame_overlap: overlapping.filter(sample => sample.second_frame_during_native_scan).length },
    baseline: statistics(samples.filter(sample => sample.phase === "baseline")),
    native_input_overlap: statistics(overlapping),
    samples, status_timeline: observed.status_changes, memory_samples: memorySamples, memory_errors: memoryErrors,
    memory_method: "Electron MemoryInfo values are KB. main and primary renderer process-lifetime peaks include startup; native worker bytes are reported separately by Rust.",
    renderer_errors: rendererErrors, test_transport_unavailable_queries: [...new Set(unsupportedActions)],
    sample_limit: 200, sample_interval_ms: 50,
  };
  assert.equal(rendererErrors.length, 0, "no renderer exception during measurement");
} catch (error) {
  report = { schema_version: 1, measured_at: new Date().toISOString(), status: "failed", run_id: request.run_id,
    case: request.case, concurrency: request.concurrency, error: error.stack ?? error.message,
    native_events: native.events, native_stderr_tail: native.stderrTail, native_stdout_tail: native.stdoutTail,
    renderer_errors: rendererErrors, automation_samples: automationSamples, memory_samples: memorySamples,
    request_counts: { native_workers: native.started ? 1 : 0, scan: scanCalls,
      model_turns: gateway?.receivedMethods.filter(method => method === "turn/start" || method === "turn/steer").length ?? 0 } };
  process.exitCode = 1;
} finally {
  const cleanupErrors = await cleanup();
  report ??= { schema_version: 1, status: "failed", error: "measurement ended without a result" };
  report.cleanup_errors = cleanupErrors;
  if (cleanupErrors.length) { report.status = "failed"; process.exitCode = 1; }
  process.removeListener("SIGINT", onSignal); process.removeListener("SIGTERM", onSignal);
  await mkdir(dirname(outputPath), { recursive: true });
  await writeFile(outputPath, JSON.stringify(report, null, 2) + "\n");
  console.log(JSON.stringify({ output: outputPath, status: report.status, samples: report.sample_counts ?? null, model_turns: report.request_counts?.model_turns ?? null }));
}
