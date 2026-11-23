import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { _electron as electron } from "playwright-core";
import * as fake from "../fake-gateway/fakeGateway.ts";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const userData = await mkdtemp(join(tmpdir(), "akra-storage-live-e2e-"));
const kernel = "local-user-live-storage", extension = "akra.storage-analysis";
let job = null, scans = 0, cancellations = 0, priorityCalls = 0, queueFull = true, revision = 1;
let delayedChildren = null, childEntered = null;
const actions = [];
const gateway = await fake.startFakeGateway(fake.uniqueTestEndpoint(), [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  kernel: { kernel: "repository-live-fixture", queries: {}, onRequest(method, params) {
    if (method === "akra/kernel/extensions/attach") return { result: { kernel, scope: "local_user", generation: "live-host" } };
    if (method === "akra/kernel/extensions/detach") return { result: {} };
    if (method === "akra/kernel/extensions/list") return { result: params.kernel === kernel ? [{ id: extension, version: 1, scope: "local_user",
      enabled: true, desired_enabled: true, compatible: true, lifecycle: "enabled", generation: "live-host", revision }] : [] };
    if (method !== "akra/kernel/extensions/call") return;
    assert.equal(params.kernel, kernel);
    assert.equal(params.extension, extension);
    const input = params.input ?? {}; actions.push(params.action);
    if (params.action === "volumes") return { result: { volumes: [{ id: "stable-volume", label: "진행 테스트 드라이브", root_path: "C:\\", kind: "ssd", supported: true }] } };
    if (params.action === "jobs") return { result: { jobs: job ? [job] : [] } };
    if (params.action === "scan") {
      scans++;
      job = { id: "running-job-" + scans, extension, action: "scan", state: "running", generation: "live-host", revision: 1, sequence: 1,
        progress: { files_scanned: 1, directories_scanned: 2, volume_id: "stable-volume" }, result: null, error: null, cleanup: "pending", created_at_ms: 1, updated_at_ms: 1 };
      return { result: job };
    }
    if (params.action === "live_children") {
      assert.equal(input.job_id, job.id);
      if (input.node_id === "n2") childEntered?.();
      return { result: { job_id: job.id, revision: 0, root_node_id: "n1", node_id: input.node_id,
        items: input.node_id === "n1" ? [{ id: "n2", parent_id: "n1", name: "cache", enumeration_complete: false }] : [],
        total_count: input.node_id === "n1" ? 1 : null, next_cursor: null, enumeration_complete: input.node_id === "n1" },
        replyAfter: input.node_id === "n2" ? delayedChildren ?? undefined : undefined };
    }
    if (params.action === "prioritize") {
      priorityCalls++;
      assert.deepEqual(input, { job_id: job.id, node_id: "n2" });
      return queueFull ? { error: { code: -32000, message: "storage_priority_queue_full" } }
        : { result: { job_id: job.id, node_id: "n2", queued: true, queue_length: 1 } };
    }
    if (params.action === "cancel") {
      cancellations++; job = { ...job, state: "cancelled", revision: 2, sequence: 2, result: null, cleanup: "complete" };
      return { result: job };
    }
    return { error: { code: -32602, message: "Unexpected live fixture action: " + params.action } };
  } },
});
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: gateway.endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;
let app, release;
try {
  app = await electron.launch({ executablePath: createRequire(import.meta.url)("electron"), args: [root, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
  const view = await app.firstWindow({ timeout: 30_000 }); view.setDefaultTimeout(15_000);
  await view.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor();
  await view.getByTestId("storage-toggle").click();
  const panel = view.getByRole("region", { name: "저장 공간 분석기", exact: true });
  await panel.getByRole("button", { name: "분석", exact: true }).click();
  const live = panel.getByRole("region", { name: "분석 중 폴더 탐색", exact: true });
  await live.locator('[data-live-node-id="n2"]').waitFor();
  if (process.env.AKRA_STORAGE_SCREENSHOT) {
    await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1440, 1000));
    await view.getByTestId("akra.storage-analysis-maximize").click();
    await view.evaluate(async () => { await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))); });
    const png = await app.evaluate(async ({ BrowserWindow }) => (await BrowserWindow.getAllWindows()[0].webContents.capturePage()).toPNG().toString("base64"));
    const { writeFile } = await import("node:fs/promises");
    await writeFile(process.env.AKRA_STORAGE_SCREENSHOT, Buffer.from(png, "base64"));
    await view.getByTestId("akra.storage-analysis-restore").click();
  }

  if (process.env.AKRA_STORAGE_METRICS) {
    await view.getByTestId("thread-entry").first().click();
    const composer = view.locator("textarea").first();
    await composer.waitFor();
    const memorySamples = [];
    const sampleMemory = async stage => {
      const processes = await app.evaluate(({ app, BrowserWindow }) => {
        const rendererPid = BrowserWindow.getAllWindows()[0].webContents.getOSProcessId();
        return app.getAppMetrics().filter(item => item.type === "Browser" || item.pid === rendererPid).map(item => ({
          role: item.type === "Browser" ? "main" : "renderer",
          working_set_kb: item.memory.workingSetSize,
          peak_working_set_kb: item.memory.peakWorkingSetSize,
          private_kb: item.memory.privateBytes ?? null,
        }));
      });
      memorySamples.push({ stage, processes });
    };
    await sampleMemory("running_before_inputs");
    const samples = [];
    await view.evaluate(() => { window.__storageLatencyFrames = []; });
    for (let index = 0; index < 3; index++) {
      const text = "저장 공간 입력 반응 fixture " + (index + 1);
      await view.evaluate(() => {
        window.__storageLatencySample = null;
        const box = document.querySelector("textarea");
        box.addEventListener("input", () => {
          const start = performance.now();
          const sample = { input_to_dom_microtask_ms: null, input_to_frame_ms: null, value: null };
          window.__storageLatencySample = sample;
          window.__storageLatencyFrames.push(sample);
          queueMicrotask(() => { sample.input_to_dom_microtask_ms = performance.now() - start; sample.value = box.value; });
          requestAnimationFrame(() => { sample.input_to_frame_ms = performance.now() - start; });
        }, { once: true });
      });
      gateway.sendExtensionChange(kernel, { extension, generation: "live-host", revision, sequence: index + 10, job_id: job.id });
      const start = performance.now();
      await composer.fill(text);
      assert.equal(await composer.inputValue(), text);
      const automationToDomMs = performance.now() - start;
      const measured = await view.evaluate(() => window.__storageLatencySample);
      assert.equal(measured.value, text);
      samples.push({ sample: index + 1, text_length: text.length, automation_to_dom_ms: automationToDomMs, input_to_dom_microtask_ms: measured.input_to_dom_microtask_ms });
      await sampleMemory("input_" + (index + 1));
    }
    await view.evaluate(() => new Promise(resolve => requestAnimationFrame(resolve)));
    const frames = await view.evaluate(() => window.__storageLatencyFrames.map(sample => sample.input_to_frame_ms));
    samples.forEach((sample, index) => { sample.input_to_next_frame_ms = frames[index]; });
    await composer.fill("");
    const versions = await app.evaluate(() => ({ electron: process.versions.electron, chrome: process.versions.chrome, node: process.versions.node }));
    const peaks = Object.fromEntries(["main", "renderer"].map(role => [role, Math.max(...memorySamples.flatMap(sample => sample.processes.filter(item => item.role === role).map(item => item.peak_working_set_kb)))]));
    const report = {
      schema_version: 1, measured_at: new Date().toISOString(), platform: process.platform, versions,
      fixture: { gateway: "fake", native_scan: false, discovered_directories: 2, progress_notification_per_sample: true, hidden_electron: true },
      latency_method: "Playwright fill start to inputValue confirmation includes automation overhead. Renderer input event to microtask DOM observation is separate. Next-frame timings are recorded separately because hidden-window requestAnimationFrame cadence is throttled.",
      memory_method: "Electron app.getAppMetrics MemoryInfo; all raw fields are reported in Kilobytes. peakWorkingSetSize is the process-lifetime peak observed at sampling, including startup.",
      excluded: ["native scanner load", "full volume scale", "GPU and utility processes", "fake Gateway test process"],
      samples, memory_samples: memorySamples, process_lifetime_peak_working_set_kb: peaks,
    };
    const { writeFile } = await import("node:fs/promises");
    await writeFile(process.env.AKRA_STORAGE_METRICS, JSON.stringify(report, null, 2) + "\n");
  }
  assert.equal(scans, 1); assert.equal(priorityCalls, 0, "mount and initial discovery do not submit priority requests");
  assert.equal(await panel.getByRole("button", { name: "이 폴더를 AI에 묻기", exact: true }).count(), 0, "pending metadata cannot be shared");
  await live.getByRole("button", { name: "cache 펼치기", exact: true }).click();
  await live.getByRole("alert").filter({ hasText: "storage_priority_queue_full" }).waitFor();
  assert.equal(priorityCalls, 1);
  await live.getByRole("button", { name: "cache 접기", exact: true }).click();
  queueFull = false;
  const entered = new Promise(resolve => { childEntered = resolve; });
  delayedChildren = new Promise(resolve => { release = resolve; });
  await live.getByRole("button", { name: "cache 펼치기", exact: true }).click();
  await entered;
  await view.getByTestId("storage-toggle").click();
  release(); release = null; delayedChildren = null; childEntered = null;
  await live.waitFor({ state: "hidden" });
  await view.getByTestId("storage-toggle").click();
  await live.locator('[data-live-node-id="n2"]').waitFor();
  assert.equal(scans, 1, "reopen reads the existing running job");
  await panel.getByRole("button", { name: "공유 분석 취소", exact: true }).click();
  await live.waitFor({ state: "hidden" });
  assert.match(await panel.innerText(), /취소됨/);
  assert.equal(await panel.locator("[data-node-id]").count(), 0, "cancelled jobs without retained results do not invent snapshots");
  assert.equal(cancellations, 1);
  assert.equal(scans, 1);
  await panel.getByRole("button", { name: "분석", exact: true }).click();
  await live.locator('[data-live-node-id="n2"]').waitFor();
  assert.equal(scans, 2, "only a new explicit scan starts the next job");
  assert.ok(!actions.includes("snapshot"), "running/cancelled jobs without results do not issue an unbound snapshot query");
  assert.ok(!actions.some(action => action.startsWith("share_")));
  assert.ok(!gateway.receivedMethods.includes("turn/start") && !gateway.receivedMethods.includes("turn/steer"));
  console.log("PASS live storage: explicit scan, pending metadata, explicit bounded priority, queue error, late close response, reopen, cancel clears live rows, explicit rescan; no model calls");
} finally {
  release?.();
  if (app) await app.close();
  await gateway.close();
  await rm(userData, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 });
}
