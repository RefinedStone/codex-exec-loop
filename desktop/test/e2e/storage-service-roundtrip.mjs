// Manual native adapter evidence. Run with AKRA_STORAGE_FIXTURE_BIN; never builds Rust or calls a model.
import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { _electron as electron } from "playwright-core";
import * as fake from "../fake-gateway/fakeGateway.ts";
import { startStorageServiceBridge, storageCliQuery } from "./fixtures/storage-service-bridge.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const bridge = await startStorageServiceBridge();
const info = bridge.info;
const userData = [await mkdtemp(join(tmpdir(), "akra-service-desktop-a-")), await mkdtemp(join(tmpdir(), "akra-service-desktop-b-"))];
const transcript = structuredClone(fake.loadReplayTable(fake.INTERRUPTED_TURN_RECORDING).get("thread/resume").result);
transcript.thread.id = info.thread_id;
// Keep one completed, empty fixture turn so the replay does not need an unrelated turns/list recording.
transcript.thread.turns = [{ id: "fixture-existing-turn", status: "completed", items: [], itemsView: "full", startedAt: 1, completedAt: 2, durationMs: 1, error: null }];
const gateway = await fake.startFakeGateway(fake.uniqueTestEndpoint(), [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  readThread: () => structuredClone(transcript),
  kernel: { kernel: "unrelated-repository-fixture", queries: {}, onRequest(method, params) {
    if (!method.startsWith("akra/kernel/extensions/")) return;
    if (method === "akra/kernel/extensions/list" && params.kernel !== info.kernel) return { result: [] };
    return bridge.forward(method, params);
  } },
});
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: gateway.endpoint, AKRA_DESKTOP_HIDDEN: "1" };
delete env.ELECTRON_RUN_AS_NODE;
const apps = [];
const launch = async directory => {
  const app = await electron.launch({ executablePath: createRequire(import.meta.url)("electron"), args: [root, ...(process.env.CI ? ["--no-sandbox"] : [])],
    env: { ...env, AKRA_DESKTOP_USER_DATA: directory } });
  apps.push(app);
  const page = await app.firstWindow({ timeout: 30_000 }); page.setDefaultTimeout(20_000);
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor();
  await page.getByTestId("thread-entry").first().click();
  await page.waitForFunction(() => { const input = document.querySelector('[data-testid="composer-input"]'); return input && !input.disabled; });
  await page.getByTestId("storage-toggle").click();
  const panel = page.getByRole("region", { name: "저장 공간 분석기", exact: true });
  await panel.getByRole("treegrid").waitFor();
  return { app, page, panel };
};
const calls = action => bridge.trace.filter(item => item.method === "akra/kernel/extensions/call" && item.params.action === action);
const fact = node => ({ snapshot_id: node.snapshot_id, revision: node.revision, node_id: node.id,
  allocated_bytes: node.allocated_bytes, numerator_bytes: node.ratio.numerator_bytes, denominator_bytes: node.ratio.denominator_bytes });
try {
  const first = await launch(userData[0]);
  await first.panel.getByRole("row").filter({ hasText: "target" }).first().waitFor();
  const pageReply = calls("list_children").find(item => item.params.input.node_id === info.snapshot.root_node_id && item.reply.result?.items?.some(node => node.name === "target"));
  assert.ok(pageReply, "Desktop must obtain the target through its real Gateway call adapter");
  const target = pageReply.reply.result.items.find(node => node.name === "target");
  const desktop = fact(target);
  assert.deepEqual(desktop, { snapshot_id: info.snapshot.id, revision: info.snapshot.revision, node_id: target.id,
    allocated_bytes: "9663676416", numerator_bytes: "9663676416", denominator_bytes: "12884901888" });
  const targetRow = first.panel.locator('[data-node-id="' + target.id + '"]');
  assert.match(await targetRow.innerText(), /75.0%/);
  assert.equal(calls("scan").length, 0, "opening the existing seeded snapshot does not scan");
  await first.panel.locator('[data-node-id="' + info.snapshot.root_node_id + '"]').click();
  await first.panel.getByRole("button", { name: "이 폴더를 AI에 묻기", exact: true }).click();
  await first.panel.getByRole("region", { name: "폴더 공유 미리보기", exact: true }).waitFor();
  await first.panel.getByRole("button", { name: "대화 입력창에 공유", exact: true }).click();
  await first.panel.getByRole("button", { name: "공유 해제", exact: true }).waitFor();
  const grant = calls("share_create").at(-1).reply.result;
  assert.equal(grant.snapshot_id, desktop.snapshot_id);
  assert.match(grant.prompt, /akra extensions call/);
  await first.page.waitForFunction(prompt => document.querySelector('[data-testid="composer-input"]')?.value === prompt, grant.prompt);
  assert.equal(await first.page.getByTestId("composer-input").inputValue(), grant.prompt);

  const query = (action, nodeId) => storageCliQuery(gateway.endpoint, info.thread_id, action, { grant_id: grant.id, node_id: nodeId });
  const tool = await query("shared_get_node", target.id);
  const children = await query("shared_list_children", info.snapshot.root_node_id);
  const hotspots = await query("shared_find_hotspots", info.snapshot.root_node_id);
  assert.equal(tool.snapshot_id, desktop.snapshot_id);
  assert.equal(tool.revision, desktop.revision);
  assert.deepEqual(fact(tool.node), desktop);
  assert.deepEqual(fact(children.items.find(item => item.node.id === target.id).node), desktop);
  assert.deepEqual(fact(hotspots.items.find(item => item.node.id === target.id).node), desktop);
  assert.ok(tool.reference.startsWith("akra-storage:"), "the evidence must be generated by the actual sharing service");
  const bytesPercent = BigInt(tool.node.ratio.numerator_bytes) * 100n / BigInt(tool.node.ratio.denominator_bytes);
  assert.equal(bytesPercent, 75n);

  // Only the answer presentation is scripted; its identity/values/reference came from the real CLI query.
  transcript.thread.turns = [{ id: "service-fixture-answer", status: "completed", itemsView: "full", startedAt: 1, completedAt: 2,
    durationMs: 1, error: null, items: [{ type: "agentMessage", id: "service-fixture-item",
      text: "공유 범위의 target을 확인했습니다. [실제 서비스 target](" + tool.reference + ")",
      phase: "final_answer", memoryCitation: null, delivery: null, questions: null }] }];
  const second = await launch(userData[1]);
  const secondTarget = second.panel.locator('[data-node-id="' + target.id + '"]');
  assert.match(await secondTarget.innerText(), /75.0%/);
  const before = await second.panel.locator('[aria-selected="true"][data-node-id]').getAttribute("data-node-id");
  assert.equal(before, info.snapshot.root_node_id, "an unclicked answer leaves the existing selection alone");
  await second.page.getByRole("button", { name: "표에서 보기 · 실제 서비스 target", exact: true }).focus();
  await second.page.keyboard.press("Enter");
  await second.page.waitForFunction(id => document.activeElement?.getAttribute("data-node-id") === id, target.id);
  assert.equal(await secondTarget.getAttribute("aria-selected"), "true");
  assert.equal(await secondTarget.getAttribute("tabindex"), "0");
  const resolved = calls("resolve_evidence").at(-1).reply.result;
  assert.equal(resolved.node_id, desktop.node_id);
  assert.equal(resolved.snapshot_id, desktop.snapshot_id);
  assert.equal(resolved.revision, desktop.revision);
  assert.deepEqual(fact(resolved.node), desktop);
  assert.equal(calls("scan").length, 0);
  assert.ok(!gateway.receivedMethods.includes("turn/start") && !gateway.receivedMethods.includes("turn/steer"));
  const evidence = {
    captured_at: new Date().toISOString(),
    fixture: "one real SQLite index with test-only 12/9/2/1 GiB metadata seeded after enumerating three empty files",
    boundary: "production Desktop Gateway adapter and production CLI verify_thread/request_over_socket -> test forwarding transport -> one registered host, native read adapter, and sharing service",
    excluded: ["12 GiB physical-file scanning", "full production Gateway authentication", "host/process recovery in this scenario", "model turns"],
    desktop, cli: fact(tool.node), cli_list_children: fact(children.items.find(item => item.node.id === target.id).node),
    cli_hotspot: fact(hotspots.items.find(item => item.node.id === target.id).node), evidence: fact(resolved.node),
    evidence_reference: tool.reference, focused_node_id: await second.page.evaluate(() => document.activeElement?.getAttribute("data-node-id")),
    displayed_ratio: "75.0%", scans_after_fixture_preparation: calls("scan").length, model_calls: 0,
  };
  if (process.env.AKRA_STORAGE_ROUNDTRIP_EVIDENCE) await writeFile(process.env.AKRA_STORAGE_ROUNDTRIP_EVIDENCE, JSON.stringify(evidence, null, 2) + "\n");
  console.log("PASS one real SQLite fixture through Desktop and CLI adapters: raw snapshot/node/numerator/denominator agree, UI75%, real service evidence keyboard focus; no model calls");
} finally {
  for (const app of apps.reverse()) await app.close();
  await gateway.close();
  await bridge.close();
  for (const directory of userData) await rm(directory, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 });
}
