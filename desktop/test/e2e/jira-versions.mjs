import assert from "node:assert/strict";
import { EventEmitter, once } from "node:events";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { _electron as electron } from "playwright-core";

const desktop = join(dirname(fileURLToPath(import.meta.url)), "../..");
const require = createRequire(import.meta.url);
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);
const table = fake.loadReplayTable(fake.APPROVAL_THREAD_LIST_RECORDING);
for (const [method, response] of fake.loadReplayTable(fake.INTERRUPTED_TURN_RECORDING)) {
  if (!table.has(method)) table.set(method, response);
}
const endpoint = fake.uniqueTestEndpoint();
const server = createServer();
const sockets = new WebSocketServer({ server });
const userData = await mkdtemp(join(tmpdir(), "akra-jira-versions-"));
const evidence = process.env.AKRA_E2E_ARTIFACT_DIR ?? join(desktopDir, "../.omo/evidence/jira-versions");
const threadId = fake.loadReplayTable(fake.APPROVAL_THREAD_LIST_RECORDING).get("thread/list").result.data[0].id;
await mkdir(evidence, { recursive: true });
const threadId = fake.loadReplayTable(fake.APPROVAL_THREAD_LIST_RECORDING).get("thread/list").result.data[0].id;
const resource = {
  kind: "jira", key: "sample.atlassian.net/ABC-123",
  url: "https://sample.atlassian.net/browse/ABC-123", title: "ABC-123",
  source: "user", firstSeenAt: "2026-11-17T00:00:00Z",
};
const changed = { [threadId]: [] };
const profileId = "a".repeat(64);
const requests = [], events = new EventEmitter();
let state = { profileId, provider: "jira", revision: 0, status: "disconnected", activeConnection: null, operation: null,
  capabilities: { nativeInput: true, legacyImport: true, keyring: true } };
let gateway;
gateway = await fake.startFakeGateway(fake.uniqueTestEndpoint(), [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  threadDocuments: { [threadId]: [resource] }, changedThreadDocuments: changed,
  onRequest(method) { if (method === "akra/context/refresh") events.emit("contextRefreshed"); },
  providerServices(method, params) {
    requests.push({ method, params });
    if (method === "akra/providerCredentials/state") return { result: structuredClone(state) };
    if (method === "akra/providerCredentials/connect") {
      assert.deepEqual(params.configuration, { siteUrl: "https://sample.atlassian.net", deployment: "cloud", email: "fixture@example.test" });
      assert.equal("token" in params, false);
      const connection = { profileId, provider: "jira", connectionId: "c".repeat(32), revision: 1 };
      state = { ...state, revision: 1, status: "connected",
        activeConnection: { reference: connection, metadata: { configuration: params.configuration, accountId: "jira-viewer", accountName: "Fixture Jira" } },
        operation: { profileId, provider: "jira", operationId: params.operationId, expectedRevision: 0, kind: "connect", status: "committed",
          connection, cleanupPending: false, legacySnapshotId: null, legacyCleanupAllowed: false, error: null },
      };
      gateway.sendProviderChange({ provider: "jira", profileId, revision: 1 });
      return { result: structuredClone(state.operation) };
    }
    if (method === "akra/providers/jira/summary") return { result: { siteUrl: "https://sample.atlassian.net", issues: [{
      key: "ABC-123", summary: "Fixture Jira issue", url: resource.url, status: "In Progress", statusCategory: "indeterminate",
      issueType: "Task", priority: null, updatedAt: "2026-11-17T00:00:00Z",
    }] } };
    if (method === "akra/providers/jira/issuePrompt") return { result: { text: "ABC-123\n" + resource.url + "\nSynthetic draft only." } };
    return { error: { code: -32601, message: "fixture unsupported" } };
  },
});
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: gateway.endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;
const app = await electron.launch({ executablePath: require("electron"), args: [desktop], env });
try {
  await app.evaluate(({ net, ipcMain }) => {
    for (const channel of ["akra:github-summary", "akra:github-repository"]) ipcMain.removeHandler(channel);
    ipcMain.handle("akra:github-summary", () => ({ login: "fixture", myPullRequests: [], reviewRequests: [] }));
    ipcMain.handle("akra:github-repository", () => null);
    globalThis.jiraOwnerFixture = { httpCalls: 0, ipc: [] };
    const original = net.fetch.bind(net);
    net.fetch = (input, init) => {
      if (String(input).includes("sample.atlassian.net")) { globalThis.jiraOwnerFixture.httpCalls++; throw new Error("Desktop Jira HTTP forbidden"); }
      return original(input, init);
    };
    for (const [channel, handler] of [
      ["akra:provider-legacy-status", async () => ({ kind: "absent" })],
      ["akra:provider-migrate-legacy", async () => ({ kind: "held", reason: "notFound" })],
      ["akra:provider-prepare-disconnect", async () => ({ kind: "prepared" })],
      ["akra:provider-cleanup-legacy", async request => ({ kind: "committed", operationId: request.operationId, cleanupPending: false })],
      ["akra:provider-open-external", async () => undefined],
    ]) {
      ipcMain.removeHandler(channel);
      ipcMain.handle(channel, async (_event, request) => { globalThis.jiraOwnerFixture.ipc.push({ channel, request }); return handler(request); });
    }
  });
  const page = await app.firstWindow();
  const capture = async name => writeFile(join(evidence, name + ".png"), Buffer.from(await app.evaluate(async ({ BrowserWindow }) =>
    (await BrowserWindow.getAllWindows()[0].webContents.capturePage()).toPNG().toString("base64")), "base64"));
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("documents-toggle").filter({ hasText: "문서 1" }).waitFor();
  await page.getByTestId("documents-toggle").click();
  await page.getByTestId("documents-item").waitFor();
  await page.getByTestId("work-toggle").click();
  await page.getByTestId("jira-toggle").click();
  await page.getByTestId("jira-connect").waitFor();
  assert.equal(await page.getByTestId("jira-panel").locator('input[type="password"]').count(), 0);
  await page.getByTestId("jira-site").fill("https://sample.atlassian.net");
  await page.getByTestId("jira-email").fill("fixture@example.test");
  const connectedRefresh = once(events, "contextRefreshed", { signal: AbortSignal.timeout(15_000) });
  await page.getByTestId("jira-connect-submit").click();
  await connectedRefresh;
  await page.getByTestId("jira-issue").waitFor();
  assert.equal(gateway.receivedMethods.some(method => method.startsWith("akra/resourceVersion/")), false);
  assert.equal(await app.evaluate(() => globalThis.jiraOwnerFixture.httpCalls), 0);
  assert.equal(await page.evaluate(() => typeof window.akra.jira), "undefined");

  await page.getByTestId("documents-toggle").click();
  changed[threadId].push(resource.key);
  await page.getByTestId("documents-refresh").click();
  await page.getByTestId("documents-changed").waitFor({ timeout: 15_000 });
  for (const width of [1440, 900]) {
    await app.evaluate(({ BrowserWindow }, width) => BrowserWindow.getAllWindows()[0].setContentSize(width, 900), width);
    await page.waitForFunction(width => innerWidth === width, width);
    await capture("changed-" + width);
  }
  await page.evaluate(() => document.addEventListener("click", event => {
    if (event.target instanceof Element && event.target.closest("a")) event.preventDefault();
  }, true));
  await page.getByTestId("documents-item").getByRole("link").click();
  await page.getByTestId("documents-changed").waitFor({ state: "detached", timeout: 10_000 });
  await capture("read-900");
  assert.ok(!gateway.receivedMethods.some(method => ["turn/start", "turn/steer"].includes(method)));
  assert.equal(requests.some(request => JSON.stringify(request.params).includes("token")), false);
  assert.equal(JSON.stringify(await app.evaluate(() => globalThis.jiraOwnerFixture.ipc)).includes("token"), false);
  console.log("PASS Gateway-owned Jira account/context, no reverse Desktop version provider, changed/mark-read, secret-free UI/IPC and no model turns");
} finally {
  const exited = once(app.process(), "close", { signal: AbortSignal.timeout(15_000) });
  for (const window of app.windows()) await window.close();
  await exited; await gateway.close(); await rm(userData, { recursive: true, force: true });
}
