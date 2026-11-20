// 사람 확인의 실제 Electron IPC·main 전용 WS·stdio 경로를 시험한다. 실제 모델 턴은 없다.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { _electron as electron } from "playwright-core";
import { CONFIRMATION_DISPLAY, confirmationReply, confirmedRequest, createConfirmationFixture } from "../fixtures/confirmationBridge.mjs";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);
const endpoint = fake.uniqueTestEndpoint();
const requests = [];
const kernel = {
  kernel: "confirmation-fixture",
  queries: {},
  onRequest(method, params) {
    const prepared = confirmationReply(method, params);
    if (prepared) return prepared;
    if (method === "akra/kernel/command_confirmed") {
      confirmedRequest(params);
      return { result: { processed: true } };
    }
    return undefined;
  },
};
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  kernel, onRequest: (method, params) => requests.push({ method, params: structuredClone(params) }),
});
const userData = await mkdtemp(join(tmpdir(), "akra-issue-confirmation-ui-"));
const fixture = await createConfirmationFixture(userData);
const env = { ...process.env, ...fixture.environment, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;
const submitted = () => requests.filter((request) => request.method === "akra/kernel/command_confirmed");
const approvals = async () => (await fixture.metadata()).filter((entry) => entry.command === "approve");
const until = async (condition) => {
  const end = Date.now() + 15_000;
  while (Date.now() < end) {
    if (await condition()) return;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error("confirmation fixture did not reach the expected state");
};
let app;
let appProcess;
let page;
let sequence = 0;
const installRendererCapture = async () => {
  await page.evaluate(() => {
    window.confirmationMessages = [];
    window.akra.onMessage((message) => window.confirmationMessages.push(message));
  });
};
const begin = async (kind = "decision") => {
  sequence++;
  await page.evaluate(({ kind, key }) => {
    window.confirmationAnswer = undefined;
    const pending = kind === "mode"
      ? window.akra.confirmIssueAutomationMode({ workspace: "/work/example-app", mode: "observe", key })
      : window.akra.confirmIssueDecision({ workspace: "/work/example-app", decisionId: "decision-1", option: "approve", note: "공유 메모", key });
    pending.then((result) => { window.confirmationAnswer = result; }, () => { window.confirmationAnswer = { failed: true }; });
  }, { kind, key: "confirmation-e2e-" + sequence });
  try {
    await until(async () => {
      if (await app.evaluate((_electron, expected) => globalThis.confirmationFixture.dialogs.length === expected, sequence)) return true;
      const finished = await page.evaluate(() => window.confirmationAnswer !== undefined);
      if (finished) throw new Error("confirmation IPC completed before opening its native dialog");
      return false;
    });
  } catch (error) {
    // 실패 지점만 남긴다. 실제 요청, 등록 id, intent, 증거와 경로는 출력하지 않는다.
    const diagnostic = {
      methods: requests.map((request) => request.method).filter((method) => method.startsWith("akra/kernel/")),
      helperCommands: (await fixture.metadata()).map((entry) => entry.command),
      renderer: await page.evaluate(() => {
        const answer = window.confirmationAnswer;
        return answer === undefined ? "pending" : answer.failed ? "failed" : answer.cancelled ? "cancelled" : answer.outcome === "unknown" ? "unknown" : "answered";
      }),
      main: await app.evaluate((_electron, expected) => ({
        cwdMatches: process.cwd() === expected.cwd,
        executableMatches: process.env.AKRA_GATEWAY_BIN === expected.executable,
        dialogCount: globalThis.confirmationFixture.dialogs.length,
      }), { cwd: fixture.cwd, executable: fixture.environment.AKRA_GATEWAY_BIN }),
    };
    console.error("confirmation diagnostic", JSON.stringify(diagnostic));
    throw error;
  }
};
const answer = (response) => app.evaluate((_electron, response) => globalThis.confirmationFixture.resolve(response), response);
const currentAnswer = async () => {
  await until(() => page.evaluate(() => window.confirmationAnswer !== undefined));
  return page.evaluate(() => window.confirmationAnswer);
};
try {
  app = await electron.launch({
    executablePath: electronPath,
    args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])],
    env, cwd: fixture.cwd,
  });
  // Playwright의 앱 dispatcher는 종료 후 폐기되므로 실제 프로세스 핸들을 미리 보관한다.
  appProcess = app.process();
  await app.evaluate(({ dialog }) => {
    globalThis.confirmationFixture = { dialogs: [], aborted: 0, resolve: null };
    dialog.showMessageBox = (parent, options) => new Promise((resolve) => {
      const fixture = globalThis.confirmationFixture;
      fixture.dialogs.push({
        parentId: parent.id, message: options.message, detail: options.detail, buttons: options.buttons,
        defaultId: options.defaultId, cancelId: options.cancelId,
      });
      let finished = false;
      const done = (response) => {
        if (finished) return;
        finished = true;
        resolve({ response, checkboxChecked: false });
      };
      fixture.resolve = done;
      options.signal.addEventListener("abort", () => { fixture.aborted++; done(1); }, { once: true });
    });
  });
  page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await installRendererCapture();

  await begin();
  const dialog = await app.evaluate(() => globalThis.confirmationFixture.dialogs[0]);
  assert.equal(dialog.message, CONFIRMATION_DISPLAY.message);
  assert.equal(dialog.detail, CONFIRMATION_DISPLAY.detail);
  assert.deepEqual(dialog.buttons, [CONFIRMATION_DISPLAY.confirm_label, "취소"]);
  assert.equal(dialog.defaultId, 1);
  assert.equal(dialog.cancelId, 1);
  assert.ok(dialog.parentId > 0);
  assert.equal((await approvals()).length, 0);
  assert.equal(submitted().length, 0);
  await answer(1);
  assert.deepEqual(await currentAnswer(), { cancelled: true });
  assert.equal((await approvals()).length, 0);
  assert.equal(submitted().length, 0);
  console.log("PASS server copy, cancel defaults, and cancellation issue no proof");

  await begin();
  assert.equal((await approvals()).length, 0);
  await answer(0);
  assert.deepEqual(await currentAnswer(), { cancelled: false, response: { processed: true } });
  assert.equal((await approvals()).length, 1);
  assert.equal(submitted().length, 1);
  assert.deepEqual(submitted()[0].params.request, {
    command: { type: "answer_issue_decision", decision_id: "decision-1", option: "approve", note: "공유 메모" },
    idempotency_key: "confirmation-e2e-2",
  });
  console.log("PASS one native approval issues and submits one decision-bound proof");

  await begin("mode");
  assert.equal((await approvals()).length, 1);
  await answer(0);
  assert.deepEqual(await currentAnswer(), { cancelled: false, response: { processed: true } });
  assert.equal((await approvals()).length, 2);
  assert.equal(submitted().length, 2);
  assert.deepEqual(submitted()[1].params.request.command, { type: "set_issue_automation_mode", mode: "observe" });
  const renderer = await page.evaluate(() => ({
    reply: window.confirmationAnswer, messages: window.confirmationMessages,
    process: typeof window.process, require: typeof window.require,
  }));
  assert.equal(renderer.process, "undefined");
  assert.equal(renderer.require, "undefined");
  assert.ok(!JSON.stringify(renderer).includes("fixture-private-"));
  assert.ok(!JSON.stringify(renderer).includes("private-session"));
  console.log("PASS mode confirmation shares the path and renderer receives no bridge secrets");

  const injected = await page.evaluate(async () => {
    try {
      await window.akra.confirmIssueDecision({ workspace: "/work/example-app", decisionId: "decision-1", option: "approve", key: "forged", confirmation: "renderer-forgery" });
      return false;
    } catch {
      return true;
    }
  });
  assert.equal(injected, true);
  assert.equal(await app.evaluate(() => globalThis.confirmationFixture.dialogs.length), sequence);
  assert.equal((await approvals()).length, 2);

  await begin();
  const beforeReload = await app.evaluate(() => globalThis.confirmationFixture.aborted);
  await page.reload();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await until(() => app.evaluate((_electron, before) => globalThis.confirmationFixture.aborted > before, beforeReload));
  await answer(0);
  assert.equal((await approvals()).length, 2);
  assert.equal(submitted().length, 2);
  await installRendererCapture();
  console.log("PASS renderer replacement invalidates the pending confirmation");

  await begin();
  const initializedBeforeDisconnect = gateway.receivedMethods.filter((method) => method === "initialize").length;
  gateway.disconnectClients();
  assert.deepEqual(await currentAnswer(), { cancelled: true });
  await answer(0);
  assert.equal((await approvals()).length, 2);
  assert.equal(submitted().length, 2);
  await until(() => gateway.receivedMethods.filter((method) => method === "initialize").length > initializedBeforeDisconnect);
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  console.log("PASS a new connection generation cannot submit the old confirmation");

  await begin();
  const exited = new Promise((resolve) => appProcess.once("exit", resolve));
  await page.close();
  let timer;
  await Promise.race([exited, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error("hidden window did not close")), 10_000); })]);
  clearTimeout(timer);
  assert.equal((await approvals()).length, 2);
  assert.equal(submitted().length, 2);
  const helperCalls = await fixture.metadata();
  assert.ok(helperCalls.filter((entry) => entry.command === "start").every((entry) => entry.argv.length === 1 && entry.argv[0].endsWith("desktop-confirmation-bridge")));
  assert.ok(!gateway.receivedMethods.includes("turn/start") && !gateway.receivedMethods.includes("turn/steer"));
  console.log("PASS window close cancels confirmation, fixed argv has no secrets, and no model turn runs");
} finally {
  if (app && appProcess && appProcess.exitCode === null) {
    const exited = new Promise((resolve) => appProcess.once("exit", resolve));
    for (const window of app.windows()) await window.close().catch(() => {});
    let timer;
    const closed = await Promise.race([exited.then(() => true), new Promise((resolve) => { timer = setTimeout(() => resolve(false), 10_000); })]);
    clearTimeout(timer);
    if (!closed) appProcess.kill();
  }
  await gateway.close();
  await rm(userData, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 });
}
