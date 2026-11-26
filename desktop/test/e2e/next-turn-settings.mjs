// Real Electron UI with a synthetic Gateway. No provider or model process is used.
import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);
const directory = mkdtempSync(join(tmpdir(), "akra-e2e-model-"));
const threadId = "01a1abfb-8f0a-7581-a8e4-c2953347f470";
const activeTurnId = "01a1abfb-98d2-7ce1-ae1a-703e023db1e0";
const modelId = "synthetic-next-model";
const turn = { id: "settings-turn-1", status: "completed", items: [], error: null };
const recording = join(directory, "settings.jsonl");
const line = (direction, message) => JSON.stringify({ direction, text: JSON.stringify(message) });
writeFileSync(recording, [
  line("client", { id: 900, method: "turn/start", params: { threadId } }),
  line("gateway", { id: 900, result: { turn } }),
  line("gateway", { method: "thread/settings/updated", params: { threadId, threadSettings: { model: modelId, effort: "high" } } }),
  line("gateway", { method: "turn/completed", params: { threadId, turn } }),
].join("\n") + "\n");

const requests = [];
const catalogRequests = [];
let unavailable = false;
const model = (id, name, efforts, hidden = false) => ({ model: id, displayName: name, hidden,
  defaultReasoningEffort: efforts[0], supportedReasoningEfforts: efforts.map(reasoningEffort => ({ reasoningEffort, description: `${reasoningEffort} 검증 수준` })) });
const gateway = await fake.startFakeGateway(fake.uniqueTestEndpoint(), [
  recording, fake.APPROVAL_THREAD_LIST_RECORDING, fake.APPROVAL_TURN_RECORDING,
], {
  onRequest(method, params) { if (method.startsWith("turn/")) requests.push({ method, params }); },
  appServer(method, params) {
    if (method === "model/list") {
      catalogRequests.push(params);
      if (unavailable) return { error: { code: -32601, message: "synthetic catalog unavailable" } };
      return { result: params.cursor === "models-page-2"
        ? { data: [model(modelId, "검증 전용 모델", ["medium", "high"])], nextCursor: null }
        : { data: [model("gpt-6-luna", "기존 대화 모델", ["low"]), model("hidden-test-model", "숨긴 모델", ["low"], true)], nextCursor: "models-page-2" } };
    }
    if (method === "turn/steer") return { result: { turnId: activeTurnId } };
    if (method === "turn/start" && requests.filter(request => request.method === "turn/start").length > 1) {
      return { result: { turn: { id: "settings-turn-2", status: "inProgress", items: [], error: null } } };
    }
    return undefined;
  },
});
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: gateway.endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: join(directory, "profile") };
delete env.ELECTRON_RUN_AS_NODE;
const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("approval").waitFor({ timeout: 15_000 });
  const settings = page.getByTestId("next-turn-settings");
  await settings.locator("summary").click();
  const modelSelect = settings.getByRole("combobox", { name: "다음 턴 모델", exact: true });
  const effortSelect = settings.getByRole("combobox", { name: "다음 턴 추론 수준", exact: true });
  await modelSelect.selectOption(modelId, { timeout: 15_000 });
  assert.equal(await effortSelect.inputValue(), "medium");
  assert.deepEqual(await effortSelect.locator("option").evaluateAll(options => options.map(option => option.value)), ["medium", "high"]);
  assert.equal(await modelSelect.locator('option[value="hidden-test-model"]').count(), 0);
  await effortSelect.selectOption("high");
  assert.equal(catalogRequests[0]?.includeHidden, false);
  assert.equal(catalogRequests[1]?.cursor, "models-page-2");
  assert.match(await settings.locator("summary").textContent(), /gpt-6-luna/);
  assert.match(await settings.textContent(), /이후 턴에도 이어집니다/);
  if (process.env.AKRA_QA_SCREENSHOT_DIR) await page.screenshot({ path: join(process.env.AKRA_QA_SCREENSHOT_DIR, "next-turn-settings.png"), animations: "disabled" });

  await page.getByTestId("composer-input").fill("진행 중인 턴에 추가 설명");
  await page.getByTestId("composer-send").click();
  await page.waitForFunction(() => document.querySelector('[data-testid="composer-input"]')?.value === "");
  assert.equal(requests.length, 1);
  assert.equal(requests[0].method, "turn/steer");
  assert.equal(requests[0].params.expectedTurnId, activeTurnId);
  assert.equal("model" in requests[0].params || "effort" in requests[0].params, false);
  assert.equal(await modelSelect.inputValue(), modelId);
  assert.equal(await effortSelect.inputValue(), "high");

  await page.getByTestId("composer-input").fill("설정을 적용할 예약 입력");
  await page.getByTestId("composer-queue").click();
  await page.getByTestId("queued-inputs").waitFor();
  assert.equal(requests.length, 1);
  await page.getByTestId("approval-accept").click();
  await page.getByTestId("queued-inputs").waitFor({ state: "detached", timeout: 15_000 });
  await page.waitForFunction(id => document.querySelector('[data-testid="next-turn-settings"] summary')?.textContent?.includes(id), modelId);
  const starts = () => requests.filter(request => request.method === "turn/start");
  assert.equal(starts().length, 1);
  assert.equal(starts()[0].params.model, modelId);
  assert.equal(starts()[0].params.effort, "high");
  assert.equal(starts()[0].params.input[0].text, "설정을 적용할 예약 입력");
  assert.deepEqual(Object.keys(starts()[0].params).sort(), ["effort", "input", "model", "threadId"]);
  assert.equal(await modelSelect.inputValue(), "");
  assert.equal(await effortSelect.inputValue(), "");

  await page.getByTestId("composer-input").fill("그다음 턴은 설정 상속");
  await page.getByTestId("composer-send").click();
  await page.waitForFunction(() => document.querySelector('[data-testid="composer-input"]')?.value === "");
  assert.equal(starts().length, 2);
  assert.equal("model" in starts()[1].params || "effort" in starts()[1].params, false);
  await modelSelect.selectOption(modelId);
  await settings.getByRole("button", { name: "현재 설정 상속", exact: true }).click();
  assert.equal(await modelSelect.inputValue(), "");
  assert.equal(await effortSelect.inputValue(), "");
  unavailable = true;
  await settings.getByRole("button", { name: "목록 새로고침", exact: true }).click();
  await settings.getByRole("status").filter({ hasText: "목록을 확인하지 못했습니다" }).waitFor();
  assert.equal(await modelSelect.isDisabled(), true);
  assert.equal(starts().length, 2);
  console.log("PASS paged supported models/efforts, pending/current separation, unchanged steer, queued start options, inheritance/reset and catalog failure; no model calls");
} finally {
  const exited = new Promise(resolve => app.process().once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise(resolve => setTimeout(resolve, 10_000))]);
  await gateway.close();
  rmSync(directory, { recursive: true, force: true, maxRetries: 5 });
}
