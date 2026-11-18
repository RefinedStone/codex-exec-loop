import assert from "node:assert/strict";
import { EventEmitter, once } from "node:events";
import { createServer } from "node:http";
import { mkdtemp, mkdir, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";
import { WebSocketServer } from "ws";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "../..");
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
const evidence = join(desktopDir, "../.omo/evidence/jira-versions");
await mkdir(evidence, { recursive: true });
const resource = {
  kind: "jira", key: "sample.atlassian.net/ABC-123",
  url: "https://sample.atlassian.net/browse/ABC-123", title: "ABC-123",
  source: "user", firstSeenAt: "2026-11-17T00:00:00Z",
};
const methods = [];
const events = new EventEmitter();
let readVersion;
let version;
let sequence = 0;
let registered = false;
let app;

sockets.on("connection", (socket) => {
  const pending = new Map();
  const resources = () => [{ ...resource, changed: Boolean(readVersion && version !== readVersion) }];
  socket.on("message", (raw) => {
    const message = JSON.parse(raw.toString());
    const respond = (result) => socket.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, result }));
    if (!message.method) {
      const original = pending.get(message.id);
      if (original !== undefined) {
        pending.delete(message.id);
        if (typeof message.result?.version === "string") {
          version = message.result.version;
          readVersion ??= version;
        }
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: original, result: { resources: resources() } }));
        events.emit("versionChecked");
      }
      return;
    }
    methods.push(message.method);
    if (message.method === "akra/resourceVersion/register") {
      registered = true;
      respond({});
      events.emit("registered");
    } else if (message.method === "akra/resourceVersion/unregister") {
      registered = false;
      respond({});
    } else if (message.method === "akra/context/refresh" && registered) {
      const id = `akra/resourceVersion/request/${++sequence}`;
      pending.set(id, message.id);
      socket.send(JSON.stringify({
        jsonrpc: "2.0", id, method: "akra/resourceVersion/current",
        params: { kind: resource.kind, key: resource.key, url: resource.url },
      }));
    } else if (message.method.startsWith("akra/context/")) {
      if (message.method === "akra/context/markRead") readVersion = version;
      respond({ resources: resources() });
      if (message.method === "akra/context/refresh") events.emit("providerSkipped");
    } else if (message.id !== undefined) {
      socket.send(JSON.stringify({
        jsonrpc: "2.0", id: message.id,
        ...(table.get(message.method) ?? { error: { code: -32601, message: "unsupported fixture method" } }),
      }));
    }
  });
});

try {
  server.listen(endpoint);
  await once(server, "listening", { signal: AbortSignal.timeout(10_000) });
  const env = {
    ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint,
    AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData,
  };
  delete env.ELECTRON_RUN_AS_NODE;
  app = await electron.launch({ executablePath: require("electron"), args: [desktopDir], env });
  await app.evaluate(({ net }) => {
    globalThis.jiraVersionFixture = { version: "2026-11-17T00:00:00.000+0000", calls: [] };
    const original = net.fetch.bind(net);
    net.fetch = async (input, init) => {
      const url = String(input);
      if (!url.startsWith("https://sample.atlassian.net/rest/api/3/")) return original(input, init);
      const fixture = globalThis.jiraVersionFixture;
      fixture.calls.push(url);
      const body = url.endsWith("/myself") ? {} : { key: "ABC-123", fields: { updated: fixture.version } };
      return new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json" } });
    };
  });
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  const skipped = once(events, "providerSkipped", { signal: AbortSignal.timeout(15_000) });
  await page.getByTestId("thread-entry").first().click();
  await skipped;
  await page.getByTestId("documents-toggle").filter({ hasText: "문서 1" }).waitFor({ timeout: 15_000 });
  await page.getByTestId("documents-toggle").click();
  const registration = once(events, "registered", { signal: AbortSignal.timeout(10_000) });
  const initialVersion = once(events, "versionChecked", { signal: AbortSignal.timeout(15_000) });
  await page.evaluate(() => window.akra.jira.connect({
    siteUrl: "https://sample.atlassian.net", deployment: "cloud",
    email: "fixture@example.test", token: "fixture-only-token",
  }));
  await registration;
  await initialVersion;
  assert(registered, "Desktop registered its connected Jira site");
  const refreshDone = once(events, "versionChecked", { signal: AbortSignal.timeout(10_000) });
  await page.getByTestId("documents-refresh").click();
  await refreshDone;
  await app.evaluate(() => { globalThis.jiraVersionFixture.version = "2026-11-17T01:00:00.000+0000"; });
  await page.getByTestId("documents-refresh").click();
  await page.getByTestId("documents-changed").waitFor({ timeout: 15_000 });
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].showInactive());
  for (const width of [1440, 900]) {
    await page.setViewportSize({ width, height: 900 });
    await page.screenshot({ path: join(evidence, `changed-${width}.png`), animations: "disabled" });
  }
  await page.evaluate(() => document.addEventListener("click", (event) => {
    if (event.target instanceof Element && event.target.closest("a")) event.preventDefault();
  }, true));
  await page.getByTestId("documents-item").getByRole("link").click();
  await page.getByTestId("documents-changed").waitFor({ state: "detached", timeout: 10_000 });
  await page.screenshot({ path: join(evidence, "read-900.png") });
  assert(!methods.includes("turn/start") && !methods.includes("turn/steer"), "no model turns");
  const calls = await app.evaluate(() => globalThis.jiraVersionFixture.calls);
  assert(calls.some((url) => url.endsWith("/issue/ABC-123?fields=updated")), "only updated requested");
  console.log("PASS Jira reverse lookup, changed indicator, mark-read, screenshots, and no model turns");
} finally {
  if (app) {
    const exited = once(app.process(), "exit", { signal: AbortSignal.timeout(10_000) });
    for (const window of app.windows()) await window.close();
    await exited;
  }
  for (const socket of sockets.clients) socket.terminate();
  await new Promise((resolve) => sockets.close(resolve));
  await new Promise((resolve) => server.close(resolve));
  await rm(userData, { recursive: true, force: true });
}
