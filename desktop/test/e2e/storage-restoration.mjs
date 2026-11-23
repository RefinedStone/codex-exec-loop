// Regression for restored scan targets and virtual aggregate rows. All responses are fixed metadata.
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { _electron as electron } from "playwright-core";
import * as fake from "../fake-gateway/fakeGateway.ts";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const G = 1024n ** 3n;
function holdRoot(state) {
  let release;
  state.rootBarrier = new Promise(resolve => { release = resolve; });
  return () => { state.rootBarrier = null; release(); };
}
async function waitForRootRequest(state) {
  const deadline = Date.now() + 15_000;
  while (!state.calls.some(call => call.action === "get_node" && call.input.snapshot_id === "scan-" + state.serial)) {
    if (Date.now() > deadline) throw new Error("restored root request was not observed");
    await new Promise(resolve => setTimeout(resolve, 20));
  }
}
async function withPanel(volumeId, run) {
  const local = "storage-restoration-fixture", generation = "host-1";
  const state = { volumeId, serial: 1, includeD: true, calls: [], scans: [], rootBarrier: null };
  const releaseInitial = volumeId === "D" ? holdRoot(state) : null;
  const snapshot = () => ({ schema_version: 1, id: "scan-" + state.serial, revision: 1,
    volume_id: state.volumeId, root_node_id: "root", state: "complete", started_at: "2026-11-23T00:00:00Z",
    finished_at: "2026-11-23T00:01:00Z", files_scanned: 2, directories_scanned: 1, issues: [] });
  const node = (id, name, kind, units, parentId) => ({ snapshot_id: snapshot().id, revision: 1, id,
    parent_id: parentId, name, kind, allocated_bytes: String(BigInt(units) * G), known_allocated_bytes: String(BigInt(units) * G),
    logical_bytes: String(BigInt(units) * G), allocation_owner_id: null,
    ratio: { numerator_bytes: String(BigInt(units) * G), denominator_bytes: parentId ? String(12n * G) : null,
      reason: parentId ? null : "root", provisional: false }, state: "complete", issues: [], protection: [], classification: [] });
  const nodes = () => [
    node("root", state.volumeId + " 결과 " + state.serial, "directory", 12, null),
    node("direct", "직속 파일", "direct_files", 8, "root"),
    node("shared", "공유 파일", "shared_files", 4, "root"),
    node("ordinary", "ordinary.bin", "file", 8, "root"),
    node("hardlink", "linked.bin", "file", 4, "root"),
  ];
  const page = items => ({ snapshot_id: snapshot().id, revision: 1, items, total_count: items.length, next_cursor: null });
  const job = () => ({ id: "job-" + state.serial, extension: "akra.storage-analysis", action: "scan",
    state: "completed", generation, revision: 1, sequence: state.serial, progress: {},
    result: snapshot(), error: null, cleanup: "pending", created_at_ms: state.serial, updated_at_ms: state.serial,
    result_expires_at_ms: Date.now() + 3_600_000 });
  const gateway = await fake.startFakeGateway(fake.uniqueTestEndpoint(), [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
    kernel: { kernel: "unrelated-repository", queries: {}, onRequest(method, params) {
      if (method === "akra/kernel/extensions/attach") return { result: { kernel: local, scope: "local_user", generation } };
      if (method === "akra/kernel/extensions/detach") return { result: {} };
      if (method === "akra/kernel/extensions/list") return { result: params.kernel === local ? [{
        id: "akra.storage-analysis", version: 1, scope: "local_user", enabled: true, compatible: true,
        desired_enabled: true, lifecycle: "enabled", generation, revision: 1,
      }] : [] };
      if (method !== "akra/kernel/extensions/call") return;
      assert.equal(params.kernel, local);
      state.calls.push(structuredClone(params));
      const input = params.input;
      if (params.action === "volumes") return { result: { volumes: [
        { id: "C", label: "C:", root_path: "C:\\", kind: "fixed", supported: true },
        ...(state.includeD ? [{ id: "D", label: "D:", root_path: "D:\\", kind: "fixed", supported: true }] : []),
      ] } };
      if (params.action === "jobs") return { result: { jobs: [job()] } };
      if (params.action === "scan") { state.scans.push(input.volume_id); state.volumeId = input.volume_id; state.serial++; return { result: job() }; }
      if (params.action === "snapshot") return { result: snapshot() };
      if (params.action === "get_node") return { result: nodes().find(item => item.id === input.node_id), replyAfter: state.rootBarrier ?? undefined };
      if (params.action === "list_children") return { result: page(nodes().filter(item => item.parent_id === input.node_id && item.kind !== "file")) };
      // Native index files belong to their actual directory, never to a virtual aggregate row.
      if (params.action === "list_files") return { result: page(nodes().filter(item => item.parent_id === input.node_id && item.kind === "file")) };
      if (params.action === "ancestors") {
        const selected = nodes().find(item => item.id === input.node_id);
        return { result: { snapshot_id: snapshot().id, node_id: input.node_id, ancestors: selected.id === "root" ? [selected] : [nodes()[0], selected] } };
      }
      return { error: { code: -32602, message: "unexpected fixture action " + params.action } };
    } },
  });
  const userData = await mkdtemp(join(tmpdir(), "akra-storage-restoration-"));
  const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: gateway.endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
  delete env.ELECTRON_RUN_AS_NODE;
  let app;
  try {
    app = await electron.launch({ executablePath: createRequire(import.meta.url)("electron"), args: [root, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
    const view = await app.firstWindow({ timeout: 30_000 }); view.setDefaultTimeout(15_000);
    await view.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor();
    await view.getByTestId("storage-toggle").click();
    const panel = view.getByRole("region", { name: "저장 공간 분석기", exact: true });
    const waitRoot = () => panel.getByRole("row").filter({ hasText: state.volumeId + " 결과 " + state.serial }).waitFor();
    if (releaseInitial) {
      await waitForRootRequest(state);
      const drive = panel.getByLabel("분석할 드라이브");
      await drive.locator('option[value="D"]').waitFor({ state: "attached" });
      assert.equal(await drive.isDisabled(), true, "the restored result must settle before choosing the next scan target");
      assert.equal(await panel.getByRole("button", { name: /^(분석|확인 중…)$/ }).isDisabled(), true, "pending restoration cannot submit a scan of the default volume");
      assert.equal(state.scans.length, 0);
      releaseInitial();
    }
    await waitRoot();
    await run({ state, gateway, local, view, panel, waitRoot });
    assert.ok(!gateway.receivedMethods.includes("turn/start") && !gateway.receivedMethods.includes("turn/steer"));
  } finally {
    releaseInitial?.();
    if (app) await app.close();
    await gateway.close();
    await rm(userData, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 });
  }
}
const failures = [];
for (const [name, initialVolume, run] of [
  ["restored volume", "D", async ({ state, gateway, local, view, panel, waitRoot }) => {
    const drive = panel.getByLabel("분석할 드라이브");
    assert.equal(await drive.inputValue(), "D", "restoring D must not silently target the first C volume");
    assert.equal(state.scans.length, 0);
    await panel.getByRole("button", { name: "다시 분석", exact: true }).click();
    await panel.getByRole("row").filter({ hasText: "D 결과 2" }).waitFor();
    assert.deepEqual(state.scans, ["D"]);
    await drive.focus(); await drive.selectOption("C");
    const previousReads = state.calls.filter(call => call.action === "snapshot").length;
    gateway.sendExtensionChange(local);
    // Wait for the actual refresh response and two renderer frames, rather than asserting the old DOM immediately.
    const deadline = Date.now() + 15_000;
    while (state.calls.filter(call => call.action === "snapshot").length === previousReads) {
      if (Date.now() > deadline) throw new Error("snapshot refresh was not observed");
      await new Promise(resolve => setTimeout(resolve, 20));
    }
    await view.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    assert.equal(await drive.inputValue(), "C", "a later refresh of the same result preserves an explicit user choice");
    assert.equal(await drive.evaluate(element => element === document.activeElement), true, "refresh leaves the drive choice focused");
    await panel.getByRole("button", { name: "다시 분석", exact: true }).click();
    await panel.getByRole("row").filter({ hasText: "C 결과 3" }).waitFor();
    assert.deepEqual(state.scans, ["D", "C"]);
    state.volumeId = "D"; state.serial++;
    const releaseReplacement = holdRoot(state);
    try {
      gateway.sendExtensionChange(local); await waitForRootRequest(state);
      await panel.getByRole("treegrid").waitFor({ state: "detached" });
      assert.equal(await drive.isDisabled(), true, "a different job also settles before accepting another target");
      assert.equal(await panel.getByRole("button", { name: /^(분석|확인 중…)$/ }).isDisabled(), true);
    } finally { releaseReplacement(); }
    await waitRoot();
    assert.equal(await drive.inputValue(), "D", "displaying a different completed job restores that job's volume");
    state.includeD = false; gateway.sendExtensionChange(local);
    await drive.locator('option[value="D"]').filter({ hasText: "현재 목록에 없음" }).waitFor({ state: "attached" });
    assert.equal(await drive.inputValue(), "D", "a missing restored drive never retargets a scan to C");
    assert.equal(await panel.getByRole("button", { name: "다시 분석", exact: true }).isDisabled(), true);
    await drive.selectOption("C");
    assert.equal(await panel.getByRole("button", { name: "다시 분석", exact: true }).isEnabled(), true);
    assert.deepEqual(state.scans, ["D", "C"]);
  }],
  ["virtual aggregate rows", "C", async ({ state, view, panel }) => {
    for (const id of ["direct", "shared"]) {
      const row = panel.locator('[data-node-id="' + id + '"]');
      await row.click();
      const detail = panel.getByRole("region", { name: "선택 항목 상세", exact: true });
      assert.equal(await detail.locator(".storage-files").count(), 0, "aggregate rows do not show a misleading empty file table");
      assert.equal(await detail.getByLabel("파일 이름 필터").count(), 0);
      assert.equal(await detail.getByLabel("파일 확장자 필터").count(), 0);
      assert.equal(await detail.getByLabel("하위 파일 포함").count(), 0);
      assert.equal(await detail.getByLabel("선택 폴더 트리맵 표시").count(), 0);
      assert.equal(await detail.getByRole("button", { name: "파일 다음 항목", exact: true }).count(), 0);
      assert.equal(await detail.getByRole("button", { name: "이 폴더를 AI에 묻기", exact: true }).count(), 0);
      assert.equal(await row.getAttribute("aria-expanded"), null, "aggregate rows are leaves");
      await row.focus(); await view.keyboard.press("ArrowRight");
      const owner = detail.getByRole("button", { name: "집계 소유 폴더로 이동", exact: true });
      await owner.click();
      await view.waitForFunction(() => document.activeElement?.getAttribute("data-node-id") === "root");
      await panel.getByRole("button", { name: "ordinary.bin", exact: true }).waitFor();
      assert.equal(state.calls.filter(call => ["list_files", "list_children"].includes(call.action) && call.input.node_id === id).length, 0);
    }
    assert.equal(state.scans.length, 0);
  }],
]) {
  try { await withPanel(initialVolume, run); console.log("PASS " + name); }
  catch (error) { failures.push(error); console.error("FAIL " + name + ": " + error); }
}
if (failures.length) throw new AggregateError(failures, "storage restoration regressions failed");
