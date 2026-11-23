import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { _electron as electron } from "playwright-core";
import * as fake from "../fake-gateway/fakeGateway.ts";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const extension = "akra.storage-analysis", kernel = "lifecycle-user-fixture";
const dirs = [await mkdtemp(join(tmpdir(), "akra-extension-life-a-")), await mkdtemp(join(tmpdir(), "akra-extension-life-b-"))];
let enabled = false, desired = false, lifecycle = "disabled", revision = 0, configureMode = "fail", attachCount = 0;
let replyBarrier = null, configureEntered = null, heldList = null;
const calls = [];
const manifest = () => ({ id: extension, version: 1, scope: "local_user", label: "저장 공간 분석기", enabled, desired_enabled: desired, compatible: true, lifecycle, revision, generation: "lifecycle-host" });
const gateway = await fake.startFakeGateway(fake.uniqueTestEndpoint(), [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  kernel: { kernel: "lifecycle-repository-fixture", queries: {}, onRequest(method, params) {
    if (method === "akra/kernel/extensions/attach") { attachCount++; return { result: { kernel, scope: "local_user", generation: "lifecycle-host" } }; }
    if (method === "akra/kernel/extensions/detach") return { result: {} };
    if (method === "akra/kernel/extensions/list") {
      if (params.kernel !== kernel) return { result: [] };
      const hold = heldList; heldList = null; hold?.entered();
      return { result: [manifest()], replyAfter: hold?.barrier };
    }
    if (method !== "akra/kernel/extensions/call") return;
    assert.equal(params.kernel, kernel); assert.equal(params.extension, extension);
    calls.push({ action: params.action, input: params.input });
    if (params.action === "configure") {
      configureEntered?.();
      if (configureMode === "fail") return { error: { code: -32000, message: "extension_store_failure: fixture disk write failed" } };
      if (params.input.expected_revision !== revision) return { error: { code: -32000, message: "extension_revision_conflict" } };
      desired = params.input.enabled; enabled = desired;
      lifecycle = desired ? "enabled" : configureMode === "disable_pending" ? "disabling" : "disabled"; revision++;
      return { result: manifest(), replyAfter: replyBarrier ?? undefined };
    }
    if (!enabled || lifecycle !== "enabled") return { error: { code: -32000, message: "extension_disabled" } };
    if (params.action === "volumes") return { result: { volumes: [{ id: "volume-fixture", label: "테스트 드라이브", root_path: "C:\\", kind: "ssd", supported: true }] } };
    if (params.action === "jobs") return { result: { jobs: [] } };
    return { error: { code: -32602, message: "unexpected lifecycle action " + params.action } };
  } },
});
const change = () => gateway.sendExtensionChange(kernel, { extension, generation: "lifecycle-host", revision, sequence: revision });
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: gateway.endpoint, AKRA_DESKTOP_HIDDEN: "1" };
delete env.ELECTRON_RUN_AS_NODE;
let app, second;
const row = page => page.getByTestId("settings-row-" + extension);
const panel = page => page.getByRole("region", { name: "저장 공간 분석기", exact: true });
const connected = page => page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
const launch = async userData => {
  const instance = await electron.launch({ executablePath: createRequire(import.meta.url)("electron"), args: [root, ...(process.env.CI ? ["--no-sandbox"] : [])], env: { ...env, AKRA_DESKTOP_USER_DATA: userData } });
  const page = await instance.firstWindow({ timeout: 30_000 }); page.setDefaultTimeout(15_000); await connected(page);
  return [instance, page];
};
const until = async check => { const end = Date.now() + 15_000; while (!await check()) { if (Date.now() > end) throw Error("fixture condition timed out"); await new Promise(resolve => setTimeout(resolve, 25)); } };
try {
  let page; [app, page] = await launch(dirs[0]);
  await page.getByTestId("extensions-manage").click();
  await row(page).getByRole("button", { name: "켜기", exact: true }).click();
  await row(page).getByRole("alert").filter({ hasText: "extension_store_failure" }).waitFor();
  assert.equal(await row(page).getByRole("button", { name: "켜기", exact: true }).getAttribute("aria-pressed"), "false");
  assert.equal(await page.getByTestId("storage-toggle").count(), 0);
  assert.equal(enabled, false);
  console.log("PASS failed configure reply leaves the extension disabled and exposes its error");

  configureMode = "normal";
  await row(page).getByRole("button", { name: "켜기", exact: true }).click();
  await row(page).getByRole("button", { name: "열기", exact: true }).waitFor();
  // Intercept the emitted module at the Electron boundary; no production test hook.
  await app.evaluate(({ session, net }) => {
    globalThis.extensionChunkFixture = { behavior: "fail", hits: 0, completed: 0, releases: [] };
    session.defaultSession.protocol.handle("file", async request => {
      const state = globalThis.extensionChunkFixture;
      if (/\/StoragePanel-[^/]+\.js(?:\?|$)/.test(new URL(request.url).pathname)) {
        state.hits++;
        if (state.behavior === "fail") return Response.error();
        if (state.behavior === "hold") await new Promise(resolve => state.releases.push(resolve));
        state.completed++;
      }
      return net.fetch(request.url, { bypassCustomProtocolHandlers: true });
    });
  });
  await row(page).getByRole("button", { name: "열기", exact: true }).click();
  await page.getByRole("button", { name: "패널 다시 열기", exact: true }).waitFor();
  assert.ok(await app.evaluate(() => globalThis.extensionChunkFixture.hits > 0), "the real lazy module request was rejected");
  await connected(page);
  assert.equal(enabled, true);
  await app.evaluate(() => { globalThis.extensionChunkFixture.behavior = "pass"; });
  await page.getByRole("button", { name: "패널 다시 열기", exact: true }).click();
  await panel(page).getByRole("button", { name: "분석", exact: true }).waitFor();
  assert.equal(calls.filter(call => call.action === "scan").length, 0);
  console.log("PASS real lazy import failure is isolated and retries without changing host state");

  const holdLazyModule = async () => {
    await app.evaluate(() => { globalThis.extensionChunkFixture.behavior = "hold"; });
    await page.reload(); await connected(page);
    if (!await panel(page).count()) await page.getByTestId("storage-toggle").click();
    await page.getByRole("status").filter({ hasText: "패널을 여는 중…" }).waitFor();
    await until(() => app.evaluate(() => globalThis.extensionChunkFixture.releases.length > 0));
  };
  const releaseLazyModule = async () => {
    await app.evaluate(() => {
      const state = globalThis.extensionChunkFixture; state.behavior = "pass";
      for (const release of state.releases.splice(0)) release();
    });
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  };
  await holdLazyModule();
  await page.getByTestId("storage-toggle").click();
  await panel(page).waitFor({ state: "hidden" });
  await releaseLazyModule();
  assert.equal(await panel(page).count(), 0, "a module resolved after close cannot reopen its panel");
  await page.getByTestId("storage-toggle").click();
  await panel(page).getByRole("button", { name: "분석", exact: true }).waitFor();
  console.log("PASS closing a pending lazy module ignores its late completion and allows an explicit reopen");

  await holdLazyModule();
  enabled = false; desired = false; lifecycle = "disabled"; revision++; change();
  await page.getByTestId("storage-toggle").waitFor({ state: "hidden" });
  await releaseLazyModule();
  assert.equal(await panel(page).count(), 0, "a module resolved after disable cannot reopen its panel");
  assert.equal(await page.getByTestId("extensions-manage").evaluate(element => element === document.activeElement), true);
  console.log("PASS disabling during lazy loading removes the panel and leaves focus at extension management");

  await page.getByTestId("extensions-manage").click();
  let releaseEnable, entered;
  replyBarrier = new Promise(resolve => { releaseEnable = resolve; });
  const enabling = new Promise(resolve => { configureEntered = resolve; });
  await row(page).getByRole("button", { name: "켜기", exact: true }).click();
  await enabling; configureEntered = null; replyBarrier = null;
  assert.equal(await row(page).getByRole("button", { name: "상태 변경 중…", exact: true }).isDisabled(), true);
  let other; [second, other] = await launch(dirs[1]);
  await other.getByTestId("extensions-manage").click();
  await row(other).getByRole("button", { name: "끄기", exact: true }).waitFor();

  let releaseCatalog;
  const listed = new Promise(resolve => { entered = resolve; });
  heldList = { entered, barrier: new Promise(resolve => { releaseCatalog = resolve; }) };
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await listed; // This response contains the enabled revision and must not win later.
  let releaseDisable;
  configureMode = "disable_pending";
  replyBarrier = new Promise(resolve => { releaseDisable = resolve; });
  const disabling = new Promise(resolve => { configureEntered = resolve; });
  await row(other).getByRole("button", { name: "끄기", exact: true }).click();
  await disabling; configureEntered = null; replyBarrier = null;
  change();
  await page.getByTestId("storage-toggle").waitFor({ state: "hidden" });
  await other.getByTestId("storage-toggle").waitFor({ state: "hidden" });
  releaseEnable();
  await row(page).getByRole("button", { name: "끄는 중…", exact: true }).waitFor();
  releaseCatalog();
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(resolve)));
  assert.equal(await row(page).getByRole("button", { name: "끄는 중…", exact: true }).isDisabled(), true);
  assert.equal(await page.getByTestId("storage-toggle").count(), 0, "late enable reply and old catalog cannot resurrect an extension");

  lifecycle = "disabled"; revision++; change(); releaseDisable();
  await row(page).getByRole("button", { name: "켜기", exact: true }).waitFor();
  await row(other).getByRole("button", { name: "켜기", exact: true }).waitFor();
  for (const view of [page, other]) assert.equal(await row(view).getByRole("button", { name: "켜기", exact: true }).getAttribute("aria-pressed"), "false");
  console.log("PASS opposite changes in two clients converge on disabled despite an older enable reply and catalog");

  configureMode = "normal";
  await row(page).getByRole("button", { name: "켜기", exact: true }).click();
  await row(page).getByRole("button", { name: "열기", exact: true }).waitFor();
  change();
  await row(other).getByRole("button", { name: "끄기", exact: true }).waitFor();
  await row(page).getByRole("button", { name: "열기", exact: true }).click();
  await panel(page).getByRole("button", { name: "분석", exact: true }).waitFor();
  await page.getByTestId("extensions-manage").click();
  configureMode = "disable_pending";
  replyBarrier = new Promise(resolve => { releaseDisable = resolve; });
  const beforeReconnect = attachCount;
  const disconnecting = new Promise(resolve => { configureEntered = resolve; });
  await row(page).getByRole("button", { name: "끄기", exact: true }).click();
  await disconnecting; configureEntered = null; replyBarrier = null;
  change();
  gateway.disconnectClients();
  lifecycle = "disabled"; revision++;
  releaseDisable();
  await until(() => attachCount >= beforeReconnect + 2);
  await connected(page); await connected(other);
  for (const view of [page, other]) {
    await row(view).getByRole("button", { name: "켜기", exact: true }).waitFor();
    assert.equal(await row(view).getByRole("button", { name: "켜기", exact: true }).getAttribute("aria-pressed"), "false");
    assert.equal(await view.getByTestId("storage-toggle").count(), 0);
    assert.equal(await panel(view).count(), 0);
  }
  assert.equal(calls.filter(call => call.action === "scan").length, 0);
  assert.ok(!gateway.receivedMethods.includes("turn/start") && !gateway.receivedMethods.includes("turn/steer"));
  console.log("PASS reconnect during disable reattaches to final host authority without reopening, rescanning, or model calls");

} finally {
  if (second) await second.close();
  if (app) { await app.evaluate(() => { for (const release of globalThis.extensionChunkFixture?.releases ?? []) release(); }).catch(() => {}); await app.close(); }
  await gateway.close();
  for (const dir of dirs) await rm(dir, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 });
}
