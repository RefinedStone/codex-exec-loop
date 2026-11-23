import assert from "node:assert/strict";
import { EventEmitter, once } from "node:events";
import { access, mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { _electron as electron } from "playwright-core";

const desktop = join(dirname(fileURLToPath(import.meta.url)), "../..");
const require = createRequire(import.meta.url);
const fake = await import(pathToFileURL(join(desktop, "test/fake-gateway/fakeGateway.ts")).href);
const userData = await mkdtemp(join(tmpdir(), "akra-linear-"));
const evidence = process.env.AKRA_E2E_ARTIFACT_DIR ?? join(desktop, "../.omo/evidence/linear");
await mkdir(evidence, { recursive: true });
const profileId = "a".repeat(64);
const connectionId = "c".repeat(32);
const canary = "fixture-native-only-secret-never-entered-in-desktop";
const ids = { one: "issue-1", two: "issue-2", teamOne: "team-a", teamTwo: "team-b" };
const issue = (id, identifier, teamId, active) => ({
  id, identifier, title: "Fixture " + identifier, url: "https://linear.app/fixture/issue/" + identifier + "/fixture",
  priority: active ? 1 : 0, updatedAt: "2026-11-18T00:00:00Z", branchName: "fixture/" + identifier.toLowerCase(),
  state: { id: active ? "state-progress" : "state-unstarted", name: active ? "In Progress" : "Todo", type: active ? "started" : "unstarted" },
  team: { id: teamId, key: identifier.split("-")[0], name: "Same team name" },
  project: { id: "project-" + identifier, name: "Same project name" },
  cycle: { id: "cycle-" + identifier, number: 1, name: null, isActive: active },
});
const issues = [issue(ids.one, "AKR-1", ids.teamOne, true), issue(ids.two, "OPS-2", ids.teamTwo, false)];
const events = new EventEmitter();
const requests = [];
let state = { profileId, provider: "linear", revision: 0, status: "disconnected", activeConnection: null, operation: null,
  capabilities: { nativeInput: true, legacyImport: true, keyring: true } };
let connects = 0, failSummary = false, holdSummary = false, emptySummary = false, pendingSummary;
const summary = () => ({ issues: emptySummary ? [] : issues, teams: [], projects: [], totalCached: emptySummary ? 0 : issues.length, limit: 30 });
const operation = (params, kind, status = "committed", error = null) => ({
  profileId, provider: "linear", operationId: params.operationId, expectedRevision: params.expectedRevision,
  kind, status, error, connection: state.activeConnection?.reference ?? null,
  cleanupPending: false, legacySnapshotId: null, legacyCleanupAllowed: false,
});
let gateway;
gateway = await fake.startFakeGateway(fake.uniqueTestEndpoint(), [
  fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING, fake.THREAD_START_RECORDING,
], {
  providerServices(method, params) {
    requests.push({ method, params });
    assert.equal(JSON.stringify(params).includes(canary), false);
    if (method === "akra/providerCredentials/state") return { result: structuredClone(state) };
    if (method === "akra/providerCredentials/connect") {
      assert.deepEqual(Object.keys(params).sort(), ["configuration", "expectedRevision", "operationId", "profileId", "provider"]);
      assert.deepEqual(params.configuration, {});
      connects++;
      if (connects === 1 || connects === 3) state.operation = operation(params, "connect", "cancelled", "cancelled");
      else {
        state = { ...state, revision: state.revision + 1, status: "connected", activeConnection: {
          reference: { profileId, provider: "linear", connectionId, revision: state.revision + 1 },
          metadata: { configuration: {}, accountId: "viewer", accountName: "Fixture User" },
        } };
        state.operation = operation(params, "connect");
      }
      gateway.sendProviderChange({ provider: "linear", profileId, revision: state.revision });
      return { result: structuredClone(state.operation) };
    }
    if (method === "akra/providerCredentials/disconnect") {
      state = { ...state, revision: state.revision + 1, status: "disconnected", activeConnection: null };
      state.operation = operation(params, "disconnect");
      gateway.sendProviderChange({ provider: "linear", profileId, revision: state.revision });
      return { result: structuredClone(state.operation) };
    }
    if (method === "akra/providerCredentials/operation") return { result: state.operation };
    if (method === "akra/providers/linear/summary") {
      assert.deepEqual(params.connection, state.activeConnection?.reference);
      assert.deepEqual(params.filters, { teamId: null, projectId: null, currentCycle: false });
      if (holdSummary) return new Promise(resolve => { pendingSummary = () => resolve({ result: summary() }); events.emit("summaryHeld"); });
      if (failSummary) return { error: { code: -32000, message: "synthetic read failure", data: { kind: "unavailable" } } };
      return { result: summary() };
    }
    if (method === "akra/providers/linear/issuePrompt") return { result: { text:
      "AKR-1\nhttps://linear.app/fixture/issue/AKR-1/fixture\nfixture/akr-1\nLatest fixture comment\nSynthetic draft only." } };
    return { error: { code: -32601, message: "fixture unsupported" } };
  },
  threadDocuments: {
    [fake.loadReplayTable(fake.APPROVAL_THREAD_LIST_RECORDING).get("thread/list").result.data[0].id]: [{
      kind: "linear", key: "fixture/AKR-1", url: issues[0].url, title: "AKR-1", source: "user", firstSeenAt: "2026-11-18T00:00:00Z",
    }],
  },
});
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: gateway.endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;
const app = await electron.launch({ executablePath: require("electron"), args: [desktop], env });
try {
  // The public IPC fixture contains no secret and never launches the real native prompt/keyring.
  await app.evaluate(({ ipcMain, net, dialog, shell }) => {
    for (const channel of ["akra:github-summary", "akra:github-repository"]) ipcMain.removeHandler(channel);
    ipcMain.handle("akra:github-summary", () => ({ login: "fixture", myPullRequests: [], reviewRequests: [] }));
    ipcMain.handle("akra:github-repository", () => null);
    globalThis.linearFixture = { folder: null, folderCalls: 0, opened: [], ipc: [], httpCalls: 0 };
    for (const [channel, handler] of [
      ["akra:provider-legacy-status", async () => ({ kind: "absent" })],
      ["akra:provider-migrate-legacy", async () => ({ kind: "held", reason: "notFound" })],
      ["akra:provider-prepare-disconnect", async () => ({ kind: "prepared" })],
      ["akra:provider-cleanup-legacy", async request => ({ kind: "committed", operationId: request.operationId, cleanupPending: false })],
      ["akra:provider-open-external", async request => { globalThis.linearFixture.opened.push(request.url); }],
    ]) {
      ipcMain.removeHandler(channel);
      ipcMain.handle(channel, async (_event, request) => { globalThis.linearFixture.ipc.push({ channel, request }); return handler(request); });
    }
    const original = net.fetch.bind(net);
    net.fetch = (input, init) => {
      if (String(input).includes("api.linear.app")) { globalThis.linearFixture.httpCalls++; throw new Error("Desktop provider HTTP forbidden"); }
      return original(input, init);
    };
    dialog.showOpenDialog = async () => {
      globalThis.linearFixture.folderCalls++;
      return { canceled: !globalThis.linearFixture.folder, filePaths: globalThis.linearFixture.folder ? [globalThis.linearFixture.folder] : [] };
    };
    shell.openExternal = async url => { globalThis.linearFixture.opened.push(url); };
  });
  const page = await app.firstWindow();
  const capture = async (name, expectedTestId, theme = null) => {
    await page.getByTestId(expectedTestId).first().waitFor({ timeout: 5_000 });
    if (theme) await page.waitForFunction(expected => {
      const style = getComputedStyle(document.documentElement);
      return document.documentElement.dataset.theme === expected && style.colorScheme === expected
        && style.getPropertyValue("--bg").trim() === (expected === "dark" ? "#111214" : "#ffffff");
    }, theme, { timeout: 5_000 });
    await page.evaluate(label => new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("Screenshot paint timed out: " + label)), 5_000);
      document.fonts.ready.then(() => {
        requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(() => {
          clearTimeout(timer);
          resolve();
        }, 0)));
      }, error => { clearTimeout(timer); reject(error); });
    }), name);
    await writeFile(join(evidence, name + ".png"), Buffer.from(await app.evaluate(async ({ BrowserWindow }) =>
      (await BrowserWindow.getAllWindows()[0].webContents.capturePage()).toPNG().toString("base64")), "base64"));
  };
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("work-toggle").click();
  await page.getByTestId("linear-toggle").click();
  await page.getByTestId("linear-connect").waitFor();
  assert.equal(await page.getByTestId("linear-panel").locator('input[type="password"]').count(), 0);
  assert.equal(await page.getByTestId("linear-key").count(), 0);
  assert.deepEqual(await page.evaluate(() => [typeof window.akra.jira, typeof window.akra.linear]), ["undefined", "undefined"]);
  await capture("disconnected", "linear-connect");
  await page.getByTestId("linear-connect-submit").click();
  await page.getByTestId("linear-error").waitFor();
  await page.getByTestId("linear-connect-submit").click();
  await page.getByTestId("linear-issue").nth(1).waitFor();
  await assert.rejects(access(join(userData, "linear-token.bin")), { code: "ENOENT" });
  assert.equal(await page.getByTestId("linear-issue").count(), 2);
  const summaryCalls = () => requests.filter(request => request.method.endsWith("/summary")).length;
  const beforeFilter = summaryCalls();
  await page.getByTestId("linear-team").selectOption(ids.teamOne);
  await page.waitForFunction(() => document.querySelectorAll('[data-testid="linear-issue"]').length === 1);
  assert.ok((await page.getByTestId("linear-issue").innerText()).includes("AKR-1"));
  await page.getByTestId("linear-project").selectOption("project-OPS-2");
  await page.getByTestId("linear-filtered-empty").waitFor();
  await capture("filtered-empty", "linear-filtered-empty");
  await page.getByTestId("linear-project").selectOption("");
  await page.getByTestId("linear-team").selectOption("");
  await page.getByTestId("linear-cycle").check();
  await page.waitForFunction(() => document.querySelectorAll('[data-testid="linear-issue"]').length === 1);
  assert.equal(summaryCalls(), beforeFilter);
  await page.getByTestId("linear-reconnect").click();
  await page.getByTestId("linear-error").waitFor();
  assert.equal(await page.getByTestId("linear-issue").count(), 1);
  assert.equal(summaryCalls(), beforeFilter);
  console.log("PASS metadata-only native connect, cancelled reconnect keeps account/cache, and filters never fetch");

  for (const width of [1440, 900]) {
    await app.evaluate(({ BrowserWindow }, width) => BrowserWindow.getAllWindows()[0].setContentSize(width, 900), width);
    await page.waitForFunction(width => innerWidth === width, width);
    for (const theme of ["light", "dark"]) {
      await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
      await capture("list-" + width + "-" + theme, "linear-issue", theme);
    }
  }
  const composer = page.getByTestId("composer-input");
  await composer.fill("Keep this draft");
  await page.getByTestId("linear-hand-off").first().click();
  await page.waitForFunction(() => !document.querySelector('[data-testid="linear-hand-off"]')?.disabled);
  assert.equal(await composer.inputValue(), "Keep this draft");
  assert.equal(await app.evaluate(() => globalThis.linearFixture.folderCalls), 1);
  await app.evaluate(() => { globalThis.linearFixture.folder = "C:/fixture-workspace"; });
  await page.getByTestId("linear-hand-off").first().click();
  await page.waitForFunction(() => document.querySelector('[data-testid="composer-input"]')?.value.includes("AKR-1"));
  const draft = await composer.inputValue();
  assert.ok(draft.includes("fixture/akr-1") && draft.includes("Latest fixture comment") && draft.includes("https://linear.app/"));
  assert.ok(draft.length <= 20_000);
  await capture("draft", "composer-input");
  assert.ok(!gateway.receivedMethods.some(method => ["thread/start", "turn/start", "turn/steer"].includes(method)));
  const folderCalls = await app.evaluate(() => globalThis.linearFixture.folderCalls);
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("linear-hand-off").first().click();
  await page.waitForFunction(() => document.querySelector('[data-testid="composer-input"]')?.value.includes("AKR-1"));
  assert.equal(await app.evaluate(() => globalThis.linearFixture.folderCalls), folderCalls);

  emptySummary = true; await page.getByTestId("linear-refresh").click(); await page.getByTestId("linear-empty").waitFor(); await capture("empty", "linear-empty");
  emptySummary = false; await page.getByTestId("linear-refresh").click(); await page.getByTestId("linear-issue").first().waitFor();
  failSummary = true; await page.getByTestId("linear-refresh").click(); await page.getByTestId("linear-error").waitFor(); await capture("fetch-error", "linear-error");
  failSummary = false; holdSummary = true;
  const held = once(events, "summaryHeld", { signal: AbortSignal.timeout(10_000) });
  await page.getByTestId("linear-refresh").click(); await held; await capture("loading", "linear-loading");
  await page.getByTestId("linear-disconnect").click(); await page.getByTestId("linear-connect").waitFor();
  pendingSummary?.();
  assert.equal(await page.getByTestId("linear-issue").count(), 0);
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("documents-toggle").filter({ hasText: "문서 1" }).waitFor();
  await page.getByTestId("documents-toggle").click(); await page.getByTestId("documents-item").waitFor();
  assert.ok((await page.getByTestId("documents-group").innerText()).startsWith("Linear (1)"));
  await capture("related-document", "documents-item");
  assert.equal(await app.evaluate(() => globalThis.linearFixture.httpCalls), 0);
  assert.equal(JSON.stringify(await app.evaluate(() => globalThis.linearFixture.ipc)).includes(canary), false);
  assert.equal((await page.content()).includes(canary), false);
  assert.equal(JSON.stringify(requests).includes(canary), false);
  assert.ok(!gateway.receivedMethods.some(method => ["turn/start", "turn/steer"].includes(method)));
  console.log("PASS Gateway-only reads, draft-only handoff, late-response fence, secret-free DOM/IPC/RPC and no model turns");
} finally {
  const exited = once(app.process(), "close", { signal: AbortSignal.timeout(15_000) });
  for (const window of app.windows()) await window.close();
  await exited; await gateway.close(); await rm(userData, { recursive: true, force: true });
}
