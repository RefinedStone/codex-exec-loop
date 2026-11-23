import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { _electron as electron } from "playwright-core";
import * as fake from "../fake-gateway/fakeGateway.ts";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const userData = await mkdtemp(join(tmpdir(), "akra-storage-e2e-"));
const localKernel = "local-user-storage-fixture", repoKernel = "repository-fixture";
const extension = "akra.storage-analysis";
const threadId = "01a1abfb-8f0a-7581-a8e4-c2953347f470";
const gib = 1024n ** 3n;
let enabled = false, revision = 0, scans = 0, grants = 0, job = null, snapshotRevision = 1, resolveBarrier = null, enteredResolve = null, invalidAllocation = false;
const calls = [];
const reference = "akra-storage:" + Buffer.from(JSON.stringify({ grant_id: "grant-1", node_id: "target", snapshot_id: "scan-1" })).toString("base64url");
const snapshot = () => ({ schema_version: 1, id: "scan-1", revision: snapshotRevision, volume_id: "volume-c-stable", root_node_id: "root", state: "complete", started_at: 1, finished_at: 2, files_scanned: 4, directories_scanned: 3, issues: [] });
const node = (id, amount, parent = "root", kind = "directory") => ({ snapshot_id: "scan-1", revision: snapshotRevision, id, parent_id: parent,
  name: ({ root: "예시 worktree", target: "target", assets: "assets", direct: "직속 파일 합계", file: "huge-cache.bin" })[id] ?? id,
  kind, allocated_bytes: (gib * BigInt(amount)).toString(), known_allocated_bytes: (gib * BigInt(amount)).toString(), logical_bytes: (gib * BigInt(amount)).toString(),
  ratio: { numerator_bytes: (gib * BigInt(amount)).toString(), denominator_bytes: parent ? (gib * 12n).toString() : null, reason: parent ? null : "root", provisional: false },
  state: "complete", issues: [], protection: id === "root" ? [{ code: "active", detail: "현재 작업 중인 저장소", observed_at: 1 }] : [], classification: [],
});
const nodes = () => [node("root", invalidAllocation ? 8 : 12, null), node("target", 9), node("assets", 2), node("direct", 1, "root", "direct_files"), node("file", 9, "target", "file")].map(item => invalidAllocation && item.id === "root" ? { ...item, allocated_bytes: "invalid", known_allocated_bytes: "invalid" } : item);
const page = items => ({ snapshot_id: "scan-1", revision: snapshotRevision, items, total_count: items.length, next_cursor: null });
const baseThread = structuredClone(fake.loadReplayTable(fake.INTERRUPTED_TURN_RECORDING).get("thread/resume").result);
baseThread.thread.turns = [{ id: "storage-answer", status: "completed", itemsView: "full", startedAt: 1, completedAt: 2, durationMs: 1, error: null, items: [
  { type: "agentMessage", id: "storage-evidence-answer", text: `target은 9 GiB로 worktree 12 GiB의 75%입니다. [target 75%](${reference})`, phase: "final_answer", memoryCitation: null, delivery: null, questions: null },
]}];
const gateway = await fake.startFakeGateway(fake.uniqueTestEndpoint(), [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  readThread: () => structuredClone(baseThread),
  kernel: { kernel: repoKernel, queries: {}, onRequest(method, params) {
    if (method === "akra/kernel/extensions/attach") return { result: { kernel: localKernel, scope: "local_user", generation: "host-1" } };
    if (method === "akra/kernel/extensions/detach") return { result: {} };
    if (method === "akra/kernel/extensions/list") return { result: params.kernel === localKernel ? [
      { id: extension, version: 1, scope: "local_user", label: "저장 공간 분석기", enabled, desired_enabled: enabled, compatible: true, lifecycle: enabled ? "enabled" : "disabled", revision, generation: "host-1" },
      { id: "example.headless", version: 1, scope: "local_user", label: "화면 없는 확장", enabled: false, desired_enabled: false, compatible: false, compatibility_reason: "호스트 API 버전이 맞지 않습니다.", lifecycle: "disabled", revision: 0, generation: "host-1" },
    ] : [] };
    if (method !== "akra/kernel/extensions/call") return;
    assert.equal(params.kernel, localKernel);
    assert.equal(params.extension, extension);
    const input = params.input ?? {}; calls.push({ action: params.action, input });
    if (params.action === "configure") {
      assert.equal(input.expected_revision, revision); enabled = input.enabled; revision++;
      return { result: { enabled, revision } };
    }
    if (!enabled) return { error: { code: -32000, message: "extension_disabled" } };
    if (params.action === "volumes") return { result: { volumes: [{ id: "volume-c-stable", label: "테스트 드라이브", root_path: "C:\\", kind: "ssd", supported: true }] } };
    if (params.action === "jobs") return { result: { jobs: job ? [job] : [] } };
    if (params.action === "scan") {
      assert.equal(input.volume_id, "volume-c-stable"); assert.equal(typeof params.idempotency_key, "string"); scans++;
      job = { id: "job-1", extension, action: "scan", state: "completed", generation: "host-1", revision: 1, sequence: 1, progress: {}, result: snapshot(), error: null, cleanup: "complete", created_at_ms: 1, updated_at_ms: 2 };
      return { result: job };
    }
    if (params.action === "snapshot") return { result: job ? snapshot() : null };
    if (params.action === "get_node") return { result: nodes().find(item => item.id === input.node_id) };
    if (params.action === "list_children") return { result: page(input.node_id === "root" ? nodes().filter(item => item.parent_id === "root") : []) };
    if (params.action === "list_files") return { result: page(input.node_id === "target" && (!input.name_filter || "huge-cache.bin".includes(input.name_filter)) ? [node("file", 9, "target", "file")] : []) };
    if (params.action === "find_hotspots") return { result: page([{ node: node("target", 9), role: "folder", reason: "1 GiB 이상", included_bytes: (gib * 9n).toString(), provisional: false }]) };
    if (params.action === "ancestors") return { result: { snapshot_id: "scan-1", node_id: input.node_id, ancestors: [node("root", 12, null), node("target", 9)] } };
    if (params.action === "share_preview") {
      assert.equal(input.thread_id, threadId);
      return { result: { id: "preview-1", snapshot_id: "scan-1", revision: snapshotRevision, node_id: input.node_id, thread_id: threadId, root: node("root", 12, null), shared_fields: ["상대 이름", "할당 바이트", "논리 바이트", "집계 상태", "보호 사유"], expires_at_ms: Date.now() + 60000 } };
    }
    if (params.action === "share_create") {
      assert.equal(input.preview_id, "preview-1"); grants++;
      return { result: { id: "grant-1", host_id: localKernel, thread_id: threadId, snapshot_id: "scan-1", revision: snapshotRevision, root_node_id: "root", expires_at_ms: Date.now() + 60000, reference, prompt: "공유한 저장 공간: grant-1, root. 이 폴더에서 큰 항목을 설명해 주세요." } };
    }
    if (params.action === "share_revoke") { grants = 0; return { result: {} }; }
    if (params.action === "resolve_evidence") {
      enteredResolve?.();
      if (!grants) return { error: { code: -32000, message: "공유가 해제되었습니다." }, replyAfter: resolveBarrier ?? undefined };
      return { result: { snapshot_id: "scan-1", revision: 1, node_id: "target", ancestors: [node("root", 12, null)], node: node("target", 9) }, replyAfter: resolveBarrier ?? undefined };
    }
    if (params.action === "cancel") return { result: job };
    return { error: { code: -32602, message: "unknown fixture action " + params.action } };
  } },
});
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: gateway.endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;
let app, secondApp;
const secondUserData = await mkdtemp(join(tmpdir(), "akra-storage-second-e2e-"));
try {
  app = await electron.launch({ executablePath: createRequire(import.meta.url)("electron"), args: [root, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
  const view = await app.firstWindow({ timeout: 30_000 });
  view.setDefaultTimeout(15_000);
  await view.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor();
  await view.getByTestId("extensions-manage").click();
  const setting = view.getByTestId("settings-row-akra.storage-analysis");
  await setting.getByRole("button", { name: "켜기", exact: true }).click();
  await setting.getByRole("button", { name: "열기", exact: true }).click();
  const panel = view.getByRole("region", { name: "저장 공간 분석기", exact: true });
  await panel.getByRole("button", { name: "분석", exact: true }).waitFor();
  assert.equal(scans, 0, "enable and mount do not start scans");
  await panel.getByRole("button", { name: "분석", exact: true }).click();
  const target = panel.locator('[data-node-id="target"]');
  await target.waitFor();
  assert.equal(scans, 1);
  assert.match(await target.innerText(), /75.0%/);
  await panel.locator('[data-node-id="root"]').focus();
  await view.keyboard.press("ArrowDown");
  assert.equal(await target.getAttribute("aria-selected"), "true");
  await panel.getByRole("button", { name: "huge-cache.bin", exact: true }).waitFor();
  await panel.locator('[data-node-id="root"]').click();
  await panel.getByLabel("선택 폴더 트리맵 표시").check();
  await panel.getByRole("button", { name: "target, 9.0 GiB, 표에서 선택", exact: true }).click();
  assert.equal(await target.getAttribute("aria-selected"), "true");
  await view.getByTestId("storage-toggle").click();
  await view.getByTestId("storage-toggle").click();
  await target.waitFor();
  assert.equal(scans, 1, "reopen only reads the current job");
  await view.getByTestId("thread-entry").first().click();
  await panel.locator('[data-node-id="root"]').click();
  const beforeShare = calls.filter(call => call.action === "share_preview" || call.action === "share_create").length;
  assert.equal(beforeShare, 0, "click and keyboard navigation do not share");
  await panel.getByRole("button", { name: "이 폴더를 AI에 묻기", exact: true }).click();
  await panel.getByRole("region", { name: "폴더 공유 미리보기" }).waitFor();
  await panel.getByRole("button", { name: "대화 입력창에 공유", exact: true }).click();
  await panel.getByRole("button", { name: "공유 해제", exact: true }).waitFor();
  assert.equal(grants, 1);
  const draft = view.locator("textarea").first();
  assert.match(await draft.inputValue(), /grant-1/);
  assert.ok(!gateway.receivedMethods.includes("turn/start") && !gateway.receivedMethods.includes("turn/steer"));
  await panel.locator('[data-node-id="assets"]').click();
  await view.getByRole("button", { name: "표에서 보기 · target 75%", exact: true }).click();
  await view.waitForFunction(() => document.querySelector('[data-node-id="target"]')?.getAttribute("aria-selected") === "true");
  assert.equal(await target.evaluate(element => element === document.activeElement), true);

  let release;
  const entered = new Promise(resolve => { enteredResolve = resolve; });
  resolveBarrier = new Promise(resolve => { release = resolve; });
  await view.getByRole("button", { name: "표에서 보기 · target 75%", exact: true }).click();
  await entered;
  await panel.locator('[data-node-id="assets"]').click();
  release(); resolveBarrier = null; enteredResolve = null;
  // A gateway refresh is sent after the released response; wait for it through the real renderer.
  await view.getByTestId("extensions-manage").click();
  await view.getByTestId("settings-row-akra.storage-analysis").getByRole("button", { name: "열기", exact: true }).click();
  assert.equal(await panel.locator('[data-node-id="assets"]').getAttribute("aria-selected"), "true", "late evidence must preserve the newer selection");

  if (process.env.AKRA_STORAGE_SCREENSHOT) {
    await view.getByRole("dialog").waitFor({ state: "hidden" });
    await view.evaluate(async () => {
      document.querySelector(".storage-panel").scrollTop = 0;
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    });
    const png = await app.evaluate(async ({ BrowserWindow }) => (await BrowserWindow.getAllWindows()[0].webContents.capturePage()).toPNG().toString("base64"));
    const { writeFile } = await import("node:fs/promises");
    await writeFile(process.env.AKRA_STORAGE_SCREENSHOT, Buffer.from(png, "base64"));
  }

  secondApp = await electron.launch({ executablePath: createRequire(import.meta.url)("electron"), args: [root, ...(process.env.CI ? ["--no-sandbox"] : [])], env: { ...env, AKRA_DESKTOP_USER_DATA: secondUserData } });
  const secondView = await secondApp.firstWindow({ timeout: 30_000 });
  secondView.setDefaultTimeout(15_000);
  await secondView.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor();
  await secondView.getByTestId("storage-toggle").click();
  await secondView.locator('[data-node-id="target"]').waitFor();
  assert.equal(scans, 1, "a second client reads the same existing job");
  assert.match(await secondView.locator('[data-node-id="target"]').innerText(), /75.0%/);

  await panel.locator('[data-node-id="root"]').click();
  await panel.getByLabel("선택 폴더 트리맵 표시").check();
  invalidAllocation = true; snapshotRevision++; revision++;
  gateway.sendExtensionChange(localKernel, { extension, generation: "host-1", revision, sequence: revision });
  await view.getByRole("button", { name: "패널 다시 열기", exact: true }).waitFor();
  await view.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor();
  invalidAllocation = false; snapshotRevision++;
  await view.getByRole("button", { name: "패널 다시 열기", exact: true }).click();
  await target.waitFor();
  assert.match(await target.innerText(), /75.0%/);
  assert.equal(scans, 1, "panel failure and retry do not restart the shared scan");

  enabled = false; revision++;
  gateway.sendExtensionChange(localKernel, { extension, generation: "host-1", revision, sequence: revision });
  await view.getByTestId("storage-toggle").waitFor({ state: "hidden" });
  await secondView.getByTestId("storage-toggle").waitFor({ state: "hidden" });
  assert.equal(scans, 1);
  await view.getByTestId("extensions-manage").click();
  await setting.getByRole("button", { name: "켜기", exact: true }).waitFor();
  assert.ok(!gateway.receivedMethods.includes("turn/start"));
  console.log("PASS storage: home enable, explicit scan, exact 75%, keyboard/tree/map, reopen, shared preview, evidence and late-response guards, two-client synchronization, panel error isolation and retry, external disable; no model calls");
} finally {
  if (secondApp) await secondApp.close();
  if (app) await app.close();
  await gateway.close();
  await rm(userData, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 });
  await rm(secondUserData, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 });
}
