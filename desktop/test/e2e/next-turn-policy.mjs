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
const directory = mkdtempSync(join(tmpdir(), "akra-e2e-policy-"));
const threadId = "01a1abfb-8f0a-7581-a8e4-c2953347f470";
const activeTurnId = "01a1abfb-98d2-7ce1-ae1a-703e023db1e0";
const workspaceWrite = { type: "workspaceWrite", writableRoots: [], networkAccess: false, excludeTmpdirEnvVar: false, excludeSlashTmp: false };
const turn = { id: "policy-turn-1", status: "completed", items: [], error: null };
const recording = join(directory, "policy.jsonl");
const line = (direction, message) => JSON.stringify({ direction, text: JSON.stringify(message) });
writeFileSync(recording, [
  line("client", { id: 900, method: "turn/start", params: { threadId } }),
  line("gateway", { id: 900, result: { turn } }),
  line("gateway", { method: "thread/settings/updated", params: { threadId, threadSettings: {
    model: "gpt-6-luna", effort: "low", approvalPolicy: "untrusted", sandboxPolicy: workspaceWrite } } }),
  line("gateway", { method: "turn/completed", params: { threadId, turn } }),
].join("\n") + "\n");

const requests = [];
// approval-turn.jsonl 의 thread/resume 응답은 approvalPolicy "untrusted", sandbox readOnly 이다.
const gateway = await fake.startFakeGateway(fake.uniqueTestEndpoint(), [
  recording, fake.APPROVAL_THREAD_LIST_RECORDING, fake.APPROVAL_TURN_RECORDING,
], {
  onRequest(method, params) { if (method.startsWith("turn/")) requests.push({ method, params }); },
  appServer(method) {
    if (method === "model/list") return { result: { data: [], nextCursor: null } };
    if (method === "turn/steer") return { result: { turnId: activeTurnId } };
    if (method === "turn/start" && requests.filter(request => request.method === "turn/start").length > 1) {
      return { result: { turn: { id: "policy-turn-2", status: "inProgress", items: [], error: null } } };
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
  const current = settings.getByTestId("next-turn-policy-current");
  await settings.locator("summary").click();
  await page.waitForFunction(() => document.querySelector('[data-testid="next-turn-policy-current"]')?.textContent?.includes("readOnly"));
  assert.match(await current.textContent(), /\(untrusted\).*\(readOnly\).*대화 열기 응답/);
  const sandboxSelect = settings.getByRole("combobox", { name: "다음 턴 sandbox 정책", exact: true });
  await sandboxSelect.selectOption("workspaceWrite");
  const confirm = settings.getByRole("button", { name: "넓은 권한 적용 확인", exact: true });
  await confirm.waitFor();
  if (process.env.AKRA_QA_SCREENSHOT_DIR) await page.screenshot({ path: join(process.env.AKRA_QA_SCREENSHOT_DIR, "next-turn-policy.png"), animations: "disabled" });

  await page.getByTestId("composer-input").fill("진행 중인 턴에 추가 설명");
  await page.getByTestId("composer-send").click();
  await page.waitForFunction(() => document.querySelector('[data-testid="composer-input"]')?.value === "");
  assert.equal(requests.length, 1);
  assert.equal(requests[0].method, "turn/steer");
  assert.equal("approvalPolicy" in requests[0].params || "sandboxPolicy" in requests[0].params, false);

  await page.getByTestId("composer-input").fill("정책을 적용할 예약 입력");
  await page.getByTestId("composer-queue").click();
  await page.getByTestId("approval-accept").click();
  const queued = page.getByTestId("queued-inputs");
  await queued.getByRole("status").filter({ hasText: "넓은 권한" }).waitFor({ timeout: 15_000 });
  const starts = () => requests.filter(request => request.method === "turn/start");
  assert.equal(starts().length, 0, "a wider policy must not start a turn before consent");

  await confirm.click();
  await queued.getByRole("button", { name: "예약 계속", exact: true }).click();
  await queued.waitFor({ state: "detached", timeout: 15_000 });
  assert.equal(starts().length, 1);
  assert.deepEqual(starts()[0].params.sandboxPolicy, workspaceWrite);
  assert.deepEqual(Object.keys(starts()[0].params).sort(), ["input", "sandboxPolicy", "threadId"]);
  await page.waitForFunction(() => document.querySelector('[data-testid="next-turn-policy-current"]')?.textContent?.includes("설정 변경 알림"));
  assert.match(await current.textContent(), /\(untrusted\).*\(workspaceWrite\)/);
  assert.equal(await sandboxSelect.inputValue(), "");

  await page.getByTestId("composer-input").fill("그다음 턴은 정책 상속");
  await page.getByTestId("composer-send").click();
  await page.waitForFunction(() => document.querySelector('[data-testid="composer-input"]')?.value === "");
  assert.equal(starts().length, 2);
  assert.equal("approvalPolicy" in starts()[1].params || "sandboxPolicy" in starts()[1].params, false);
  console.log("PASS current policy and source, consent before a wider sandbox, policy only in the next turn/start, unchanged steer and inheritance; no model calls");
} finally {
  const exited = new Promise(resolve => app.process().once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise(resolve => setTimeout(resolve, 10_000))]);
  await gateway.close();
  rmSync(directory, { recursive: true, force: true, maxRetries: 5 });
}
