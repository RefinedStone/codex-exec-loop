// A08: 실제 Electron, 가짜 Gateway와 합성 공식 프로토콜만 사용한다. Codex 모델 호출 없음.
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);
const root = mkdtempSync(join(tmpdir(), "akra-e2e-compact-"));
const recorded = readFileSync(fake.APPROVAL_TURN_RECORDING, "utf8").trim().split(/\r?\n/).map(line => JSON.parse(JSON.parse(line).text));
const resumed = recorded.find(message => message.result?.thread)?.result;
const baseline = recorded.find(message => message.method === "turn/completed")?.params.turn;
const threadId = resumed.thread.id;
const compactItem = { type: "contextCompaction", id: "e2e-compact-item" };
const turn = (id, status) => ({ id, status, items: status === "inProgress" ? [] : [compactItem], itemsView: "full", error: null });
const recoveryTurn = turn("e2e-recovered-compact", "completed");
const successTurn = turn("e2e-success-compact", "completed");
const recording = join(root, "compact.jsonl");
const record = (direction, message) => JSON.stringify({ direction, text: JSON.stringify({ jsonrpc: "2.0", ...message }) });
writeFileSync(recording, [
  record("client", { id: 100, method: "thread/compact/start", params: { threadId } }),
  record("gateway", { id: 100, result: {} }),
  record("gateway", { method: "turn/started", params: { threadId, turn: turn(successTurn.id, "inProgress") } }),
  record("gateway", { method: "item/started", params: { threadId, turnId: successTurn.id, item: compactItem } }),
  record("gateway", { method: "item/completed", params: { threadId, turnId: successTurn.id, item: compactItem } }),
  record("gateway", { method: "turn/completed", params: { threadId, turn: successTurn } }),
].join("\n") + "\n");
let mode = "accepted";
let recoveryRead = false;
const requests = [];
const gateway = await fake.startFakeGateway(fake.uniqueTestEndpoint(), [fake.APPROVAL_THREAD_LIST_RECORDING, fake.APPROVAL_TURN_RECORDING, recording], {
  appServer(method, params) {
    if (method === "thread/compact/start" || method === "thread/turns/list" || method.startsWith("turn/")) requests.push({ method, params });
    if (method === "thread/compact/start") {
      if (mode === "accepted") return { result: {} };
      if (mode === "failed") return { error: { code: -32602, message: "synthetic invalid thread state" } };
      if (mode === "unsupported") return { error: { code: -32601, message: "synthetic unsupported method" } };
    }
    if (method === "thread/turns/list" && params.limit === 20) {
      recoveryRead = true;
      return { result: { data: [recoveryTurn, baseline], nextCursor: null, backwardsCursor: null } };
    }
    if (method === "thread/resume" && recoveryRead) return { result: { ...resumed, thread: { ...resumed.thread, status: { type: "idle" }, turns: [baseline, recoveryTurn] } } };
    return undefined;
  },
});
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: gateway.endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: join(root, "user-data") };
delete env.ELECTRON_RUN_AS_NODE;
const app = await electron.launch({ executablePath: createRequire(import.meta.url)("electron"), args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("approval").waitFor({ timeout: 15_000 });
  assert.equal(await page.getByTestId("compact-start").isDisabled(), true, "active turn cannot compact");
  assert.equal(requests.filter(request => request.method === "thread/compact/start").length, 0);
  await page.getByTestId("approval-accept").click();
  await page.waitForFunction(() => !document.querySelector('[data-testid="compact-start"]')?.hasAttribute("disabled"));
  // 시간 초과용 가짜 RAF를 설치하기 전에 실제 렌더링 화면을 기록한다.
  if (process.env.AKRA_E2E_ARTIFACT_DIR) {
    // 다른 hidden Windows 화면 시험과 같은 main capture 경계를 사용한다.
    const png = await app.evaluate(async ({ BrowserWindow }) =>
      (await BrowserWindow.getAllWindows()[0].webContents.capturePage(undefined, { stayHidden: true })).toPNG().toString("base64"));
    writeFileSync(join(process.env.AKRA_E2E_ARTIFACT_DIR, "thread-compaction.png"), Buffer.from(png, "base64"));
  }
  await page.clock.install();
  await page.getByTestId("compact-start").click();
  const status = page.getByTestId("compact-status");
  await page.locator('[data-testid="compact-status"][data-phase="running"]').waitFor();
  assert.equal(await page.getByTestId("compact-start").isDisabled(), true);
  await page.getByTestId("composer-input").fill("압축 동안 보내면 안 되는 입력");
  await page.getByTestId("composer-send").click();
  await page.getByText("문맥 압축의 결과를 확인한 뒤 입력을 보내세요.", { exact: true }).waitFor();
  assert.equal(requests.some(request => request.method === "turn/start" || request.method === "turn/steer"), false);
  await page.clock.fastForward(180_001);
  await page.locator('[data-testid="compact-status"][data-phase="unknown"]').waitFor();
  await page.getByTestId("compact-refresh").click();
  await page.locator('[data-testid="compact-status"][data-phase="completed"]').waitFor();
  assert.equal(requests.filter(request => request.method === "thread/compact/start").length, 1, "read recovery must not retry compaction");
  assert.deepEqual(requests.find(request => request.method === "thread/turns/list" && request.params.limit === 20)?.params,
    { threadId, limit: 20, sortDirection: "desc", itemsView: "full" });
  mode = "success";
  await page.getByTestId("compact-start").click();
  await page.locator(`[data-turn-id="${successTurn.id}"][data-status="completed"]`).waitFor();
  await page.locator('[data-testid="compact-status"][data-phase="completed"]').waitFor();
  assert.equal(requests.filter(request => request.method === "thread/compact/start").length, 2);
  mode = "failed";
  await page.getByTestId("compact-start").click();
  await page.locator('[data-testid="compact-status"][data-phase="failed"]').waitFor();
  assert.ok((await status.textContent()).includes("거절"));
  mode = "unsupported";
  await page.getByTestId("compact-start").click();
  await page.locator('[data-testid="compact-status"][data-phase="unsupported"]').waitFor();
  assert.equal(await page.getByTestId("compact-start").isDisabled(), true);
  console.log("PASS active-turn guard, acceptance, input exclusion, unknown/read recovery, completion, failure and unsupported UI");
} finally {
  await app.close();
  await gateway.close();
  rmSync(root, { recursive: true, force: true, maxRetries: 5 });
}
