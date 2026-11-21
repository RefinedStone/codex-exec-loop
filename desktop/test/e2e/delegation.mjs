import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { _electron as electron } from "playwright-core";

import * as fake from "../fake-gateway/fakeGateway.ts";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const endpoint = fake.uniqueTestEndpoint();
const userData = await mkdtemp(join(tmpdir(), "akra-delegation-"));
const instruction = "Summarize the fixture.\nKeep line breaks.";
const record = (id, state) => ({
  id, state, caller: "akra-desktop", delivery: "observed", phase: "working",
  reason: null, conversation_url: null, outcome_unknown: false, created_at: 1795219200000, updated_at: 1795219200000,
});
const finished = record("fixture-finished", "succeeded");
const running = record("fixture-running", "awaiting_human");
running.reason = "login_required";
const records = new Map([[finished.id, finished], [running.id, running]]);
const submissions = [];
const cancellations = [];
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  kernel: {
    kernel: "fixture-repo",
    queries: {},
    onRequest(method, params) {
      if (method === "akra/kernel/query") {
        switch (params.query?.type) {
          case "delegations":
            return { result: { type: "delegations", delegations: [...records.values()] } };
          case "delegation":
            return { result: { type: "delegation", delegation: records.get(params.query.id) ?? null } };
          case "delegation_result":
            return { result: { type: "delegation_result", text: "First line.\nSecond line." } };
        }
      }
      if (method === "akra/kernel/command") {
        const { command } = params;
        if (command?.type === "submit_delegation") {
          submissions.push(command);
          if (submissions.length === 1) return { result: { processed: true, failure: "fixture request failed" } };
          const delegation = record("fixture-submitted", "queued");
          records.set(delegation.id, delegation);
          return { result: { processed: true, result: { type: "delegation_submitted", delegation } } };
        }
        if (command?.type === "cancel_delegation") {
          cancellations.push(command.id);
          const delegation = record(command.id, "cancelled");
          records.set(delegation.id, delegation);
          return { result: { processed: true, result: { type: "delegation_cancelled", delegation } } };
        }
      }
    },
  },
});

const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;
delete env.AKRA_DESKTOP_DELEGATED_CHAT;

let app;
try {
  app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  assert.equal(await page.getByTestId("delegation-toggle").count(), 0);

  // Replace the login handler in this Electron test process before reloading the renderer.
  // This uses the real renderer and sandboxed preload bridge without creating a ChatGPT view.
  await app.evaluate(({ ipcMain }) => {
    ipcMain.removeHandler("akra:delegated-chat-enabled");
    ipcMain.removeHandler("akra:delegated-chat-open");
    ipcMain.handle("akra:delegated-chat-enabled", () => true);
    globalThis.delegationLoginClicks = 0;
    ipcMain.handle("akra:delegated-chat-open", () => ++globalThis.delegationLoginClicks === 1);
  });
  await page.reload();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("delegation-toggle").click();
  const panel = page.getByTestId("delegation-panel");
  const detail = panel.getByTestId("delegation-detail");
  const input = panel.getByLabel("위임 지시");
  const submit = panel.getByRole("button", { name: "텍스트 위임" });
  const login = panel.getByRole("button", { name: "ChatGPT 로그인 열기" });

  await panel.getByRole("button", { name: "fixture-fini" }).click();
  await detail.getByTestId("delegation-result").waitFor();
  assert.equal(await detail.getByTestId("delegation-result").textContent(), "First line.\nSecond line.");
  assert.equal(await detail.getByRole("button", { name: "위임 취소" }).count(), 0);
  assert.equal(await app.evaluate(() => globalThis.delegationLoginClicks), 0);
  assert.equal(await submit.isDisabled(), true);
  console.log("PASS list, completed detail, multiline result, and no automatic login");

  await panel.getByRole("button", { name: "fixture-runn" }).click();
  await detail.getByText("login_required").waitFor();
  await login.click();
  await login.click();
  await panel.getByRole("alert").filter({ hasText: "로그인을 열 수 없습니다" }).waitFor();
  assert.equal(await app.evaluate(() => globalThis.delegationLoginClicks), 2);
  await detail.getByRole("button", { name: "위임 취소" }).click();
  await detail.getByText("취소됨", { exact: false }).waitFor();
  assert.deepEqual(cancellations, [running.id]);
  console.log("PASS explicit login, login error, and cancellation");

  await input.fill(instruction);
  await submit.click();
  await panel.getByText("fixture request failed").waitFor();
  assert.equal(await input.inputValue(), instruction);
  await submit.click();
  await detail.getByText("fixture-submitted", { exact: true }).waitFor();
  assert.equal(await input.inputValue(), "");
  assert.equal(submissions.length, 2);
  assert.equal(submissions[0].instruction, instruction);
  assert.ok(submissions[0].idempotency_key);
  assert.equal(submissions[0].idempotency_key, submissions[1].idempotency_key);
  assert.ok(!gateway.receivedMethods.some((method) => method === "turn/start" || method === "turn/steer"));
  console.log("PASS text submit, failure preserves draft, and retry reuses the request key");
  if (process.env.AKRA_E2E_DELEGATION_SCREENSHOT) {
    const box = await panel.boundingBox();
    assert.ok(box);
    const window = await app.browserWindow(page);
    const png = await window.evaluate(async (browserWindow, box) => {
      const image = await browserWindow.webContents.capturePage(
        { x: Math.round(box.x), y: Math.round(box.y), width: Math.round(box.width), height: Math.round(box.height) },
        { stayHidden: true },
      );
      if (image.isEmpty()) throw new Error("Delegation panel capture is empty");
      return image.toPNG().toString("base64");
    }, box);
    await writeFile(process.env.AKRA_E2E_DELEGATION_SCREENSHOT, Buffer.from(png, "base64"));
  }
  console.log(await panel.innerText());
} finally {
  if (app) await app.close();
  await gateway.close();
  await rm(userData, { recursive: true, force: true });
}
