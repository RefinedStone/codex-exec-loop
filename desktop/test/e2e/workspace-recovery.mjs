// #2834: real Electron UI, synthetic Gateway inventory, isolated userData, no model turns.
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);
const userData = mkdtempSync(join(tmpdir(), "akra-e2e-workspace-recovery-"));
const endpoint = fake.uniqueTestEndpoint();
const entry = (requestId, reasons) => ({
  record: {
    requestId, repositoryDir: "/fixture/repository",
    workspaceDir: `/fixture/conversation-workspaces/${requestId}`,
    branch: `akra-conversation/${requestId}`, baseCommit: "a".repeat(40),
    state: "prepared", reason: null,
  },
  currentHead: reasons.includes("workspace_missing") ? null : "a".repeat(40),
  reasons, cleanupAllowed: false,
});
const first = entry("allocation-1", ["tracked_changes", "thread_ownership_unconfirmed", "remote_retention_unconfirmed", "untracked_files_unconfirmed"]);
const second = entry("allocation-2", ["workspace_missing", "thread_ownership_unconfirmed"]);
const requests = [];
let malformed = false;
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.BASIC_RECORDING], {
  appServer(method, params) {
    if (!method.startsWith("akra/conversationWorkspace/")) return undefined;
    requests.push({ method, params });
    if (malformed) return { result: { items: [{ ...first, cleanupAllowed: true }], nextAfter: null } };
    return { result: params.after
      ? { items: [second], nextAfter: null }
      : { items: [first], nextAfter: first.record.requestId } };
  },
});
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;
const app = await electron.launch({ executablePath: createRequire(import.meta.url)("electron"), args: [desktopDir], env });
try {
  const page = await app.firstWindow();
  page.setDefaultTimeout(15_000);
  const selectRepository = async () => {
    await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
    await page.getByTestId("thread-entry").first().waitFor({ timeout: 15_000 });
    await page.locator(".home-pickers .picker > button").click();
    await page.locator(".home-pickers .picker [role=option]").filter({ hasNotText: "폴더 고르기" }).first().click();
    await page.getByRole("button", { name: "격리 공간 복구 목록", exact: true }).click();
  };
  const resize = (width, height) => app.evaluate(({ BrowserWindow }, size) => {
    const window = BrowserWindow.getAllWindows()[0];
    window.setContentSize(size.width, size.height);
    window.showInactive();
  }, { width, height });
  await resize(1280, 800);
  await selectRepository();
  const rows = page.getByTestId("workspace-recovery-entry");
  await rows.filter({ hasText: first.record.requestId }).waitFor();
  assert.equal(await rows.count(), 1);
  assert.equal(await page.getByTestId("composer-input").count(), 1);
  assert.match(await rows.innerText(), /커밋되지 않은 파일 변경/);
  assert.match(await rows.innerText(), /대화 소유권/);
  assert.match(await rows.innerText(), /원격 보관 상태/);
  assert.match(await rows.innerText(), /추적하지 않는 파일/);
  assert.equal(await rows.getByRole("button").count(), 0);
  console.log("PASS dirty and unconfirmed allocations show preservation reasons without cleanup actions");

  const shots = process.env.AKRA_QA_SCREENSHOT_DIR;
  if (shots) {
    mkdirSync(shots, { recursive: true });
    await page.screenshot({ path: join(shots, "workspace-recovery-desktop.png") });
    await page.getByTitle(/^사이드바/).click();
    await resize(560, 760);
    await page.screenshot({ path: join(shots, "workspace-recovery-narrow.png") });
    await resize(1280, 800);
    await page.getByTitle(/^사이드바/).click();
  }
  await page.getByRole("button", { name: "다음 공간", exact: true }).click();
  await rows.filter({ hasText: second.record.requestId }).waitFor();
  assert.equal(await rows.count(), 1);
  assert.match(await rows.innerText(), /작업 공간 경로가 없습니다/);
  assert.equal(requests.at(-1).params.after, first.record.requestId);
  console.log("PASS inventory pagination preserves the server cursor and explains missing paths");

  malformed = true;
  await page.getByRole("button", { name: "처음부터 새로 읽기", exact: true }).click();
  await page.getByRole("alert").waitFor();
  assert.equal(await rows.count(), 0);
  assert.match(await page.getByRole("alert").innerText(), /응답을 검증하지 못했습니다/);
  malformed = false;
  await page.getByRole("button", { name: "처음부터 새로 읽기", exact: true }).click();
  await rows.filter({ hasText: first.record.requestId }).waitFor();
  console.log("PASS malformed cleanup authorization fails closed and explicit refresh recovers");

  await page.reload();
  await selectRepository();
  await rows.filter({ hasText: first.record.requestId }).waitFor();
  assert.equal(await rows.count(), 1);
  assert.equal(requests.at(-1).params.after, null);
  assert(requests.every(request => request.method === "akra/conversationWorkspace/inventory"));
  assert(!gateway.receivedMethods.some(method => method.startsWith("turn/") || method === "thread/start"));
  console.log("PASS reload reads durable allocation inventory without preparing, deleting, or starting turns");
} finally {
  await app.close();
  await gateway.close();
  rmSync(userData, { recursive: true, force: true, maxRetries: 5 });
}
