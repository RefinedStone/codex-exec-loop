import assert from "node:assert/strict";
import { EventEmitter, once } from "node:events";
import { createServer } from "node:http";
import { access, mkdtemp, mkdir, readFile, rm } from "node:fs/promises";
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
const ids = {
  one: "11111111-1111-4111-8111-111111111111", two: "22222222-2222-4222-8222-222222222222",
  teamOne: "33333333-3333-4333-8333-333333333333", teamTwo: "44444444-4444-4444-8444-444444444444",
};
const issue = (id, identifier, teamId, active) => ({
  id, identifier, title: `Fixture ${identifier}`, url: `https://linear.app/fixture/issue/${identifier}/fixture`,
  priority: active ? 1 : 0, updatedAt: "2026-11-18T00:00:00Z", branchName: `fixture/${identifier.toLowerCase()}`,
  state: { id: active ? "state-progress" : "state-unstarted", name: active ? "In Progress" : "Todo", type: active ? "started" : "unstarted" },
  team: { id: teamId, key: identifier.split("-")[0], name: "Same team name" },
  project: { id: `project-${identifier}`, name: "Same project name" },
  cycle: { id: `cycle-${identifier}`, number: 1, name: null, isActive: active },
});
const issues = [issue(ids.one, "AKR-1", ids.teamOne, true), issue(ids.two, "OPS-2", ids.teamTwo, false)];
const events = new EventEmitter();
const queries = [];
let failSummary = false;
let holdSummary = false;
let emptySummary = false;
let pendingSummary;
const api = createServer(async (request, response) => {
  let body = "";
  for await (const chunk of request) body += chunk;
  const payload = JSON.parse(body);
  queries.push(payload);
  assert.equal(request.method, "POST");
  assert.ok(!/\bmutation\b/.test(payload.query));
  const send = (value) => { response.writeHead(200, { "content-type": "application/json" }); response.end(JSON.stringify(value)); };
  if (request.headers.authorization !== "fixture-linear-key") return send({ errors: [{ message: "Unauthenticated" }] });
  if (payload.query.includes("assignedIssues")) {
    if (holdSummary) { pendingSummary = () => send({ data: { viewer: { assignedIssues: { nodes: issues } } } }); events.emit("summaryHeld"); return; }
    if (failSummary) return send({ errors: [{ message: "Fixture fetch failure" }] });
    return send({ data: { viewer: { id: "viewer", name: "Fixture User", assignedIssues: { nodes: emptySummary ? [] : issues } } } });
  }
  if (payload.query.includes("comments")) {
    const selected = issues.find(item => item.id === payload.variables.id);
    return send({ data: { issue: { ...selected, description: "## Fixture description\nRead only.",
      comments: { nodes: [{ body: "Latest fixture comment", createdAt: "2026-11-18T00:00:00Z", user: { name: "Fixture Author" } }], pageInfo: { hasNextPage: false } },
    } } });
  }
  return send({ data: { viewer: { id: "viewer", name: "Fixture User" } } });
});
api.listen(0, "127.0.0.1");
await once(api, "listening", { signal: AbortSignal.timeout(10_000) });
while (api.address().port <= 10240) {
  await new Promise(resolve => api.close(resolve));
  api.listen(0, "127.0.0.1");
  await once(api, "listening", { signal: AbortSignal.timeout(10_000) });
}
const apiUrl = `http://127.0.0.1:${api.address().port}/graphql`;
const gateway = await fake.startFakeGateway(fake.uniqueTestEndpoint(), [
  fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING, fake.THREAD_START_RECORDING,
], { threadDocuments: {
  [fake.loadReplayTable(fake.APPROVAL_THREAD_LIST_RECORDING).get("thread/list").result.data[0].id]: [{
    kind: "linear", key: "fixture/AKR-1", url: issues[0].url, title: "AKR-1", source: "user", firstSeenAt: "2026-11-18T00:00:00Z",
  }],
} });
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: gateway.endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;
const app = await electron.launch({ executablePath: require("electron"), args: [desktop], env });
try {
  await app.evaluate(({ net, dialog, shell }, apiUrl) => {
    const original = net.fetch.bind(net);
    net.fetch = (input, init) => original(String(input) === "https://api.linear.app/graphql" ? apiUrl : input, init);
    globalThis.linearFixture = { folder: null, folderCalls: 0, opened: [] };
    dialog.showOpenDialog = async () => {
      globalThis.linearFixture.folderCalls++;
      return { canceled: !globalThis.linearFixture.folder, filePaths: globalThis.linearFixture.folder ? [globalThis.linearFixture.folder] : [] };
    };
    shell.openExternal = async url => { globalThis.linearFixture.opened.push(url); };
  }, apiUrl);
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].showInactive());
  await page.getByTestId("work-toggle").click();
  await page.getByTestId("linear-toggle").click();
  await page.getByTestId("linear-connect").waitFor();
  await page.screenshot({ path: join(evidence, "disconnected.png") });
  await page.getByTestId("linear-key").fill("invalid");
  await page.getByTestId("linear-connect-submit").click();
  await page.getByTestId("linear-error").waitFor();
  await assert.rejects(access(join(userData, "linear-token.bin")), { code: "ENOENT" });
  await page.screenshot({ path: join(evidence, "invalid-key.png") });
  await page.getByTestId("linear-key").fill("fixture-linear-key");
  await page.getByTestId("linear-connect-submit").click();
  await page.getByTestId("linear-issue").nth(1).waitFor();
  assert.equal((await readFile(join(userData, "linear-token.bin"))).includes(Buffer.from("fixture-linear-key")), false);
  assert.equal(await page.getByTestId("linear-issue").count(), 2);
  console.log("PASS Linear invalid-key rejection, encrypted storage and issue list");
  const beforeFilter = queries.length;
  await page.getByTestId("linear-team").selectOption(ids.teamOne);
  await page.waitForFunction(() => document.querySelectorAll('[data-testid="linear-issue"]').length === 1);
  assert.ok((await page.getByTestId("linear-issue").innerText()).includes("AKR-1"));
  assert.equal(queries.length, beforeFilter);
  await page.getByTestId("linear-project").selectOption("project-OPS-2");
  await page.getByTestId("linear-filtered-empty").waitFor();
  await page.screenshot({ path: join(evidence, "filtered-empty.png") });
  await page.getByTestId("linear-project").selectOption("");
  await page.getByTestId("linear-team").selectOption("");
  await page.getByTestId("linear-cycle").check();
  await page.waitForFunction(() => document.querySelectorAll('[data-testid="linear-issue"]').length === 1);
  assert.equal(queries.length, beforeFilter);
  console.log("PASS Linear filters use stable IDs and cached data");
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].showInactive());
  for (const width of [1440, 900]) {
    await app.evaluate(({ BrowserWindow }, width) => BrowserWindow.getAllWindows()[0].setContentSize(width, 900), width);
    await page.waitForFunction(width => innerWidth === width, width);
    for (const theme of ["light", "dark"]) {
      await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
      await page.screenshot({ path: join(evidence, `list-${width}-${theme}.png`), animations: "disabled" });
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
  assert.ok(!gateway.receivedMethods.includes("thread/start") && !gateway.receivedMethods.includes("turn/start") && !gateway.receivedMethods.includes("turn/steer"));
  await page.screenshot({ path: join(evidence, "draft.png") });
  console.log("PASS Linear folder cancellation preserves draft and handoff never sends");
  const folderCalls = await app.evaluate(() => globalThis.linearFixture.folderCalls);
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("linear-hand-off").first().click();
  await page.waitForFunction(() => document.querySelector('[data-testid="composer-input"]')?.value.includes("AKR-1"));
  assert.equal(await app.evaluate(() => globalThis.linearFixture.folderCalls), folderCalls);
  console.log("PASS Linear handoff reuses the open local thread folder");
  emptySummary = true;
  await page.getByTestId("linear-refresh").click();
  await page.getByTestId("linear-empty").waitFor();
  await page.screenshot({ path: join(evidence, "empty.png") });
  emptySummary = false;
  await page.getByTestId("linear-refresh").click();
  await page.getByTestId("linear-issue").first().waitFor();
  failSummary = true;
  await page.getByTestId("linear-refresh").click();
  await page.getByTestId("linear-error").waitFor();
  await page.screenshot({ path: join(evidence, "fetch-error.png") });
  failSummary = false;
  holdSummary = true;
  const held = once(events, "summaryHeld", { signal: AbortSignal.timeout(10_000) });
  await page.getByTestId("linear-refresh").click();
  await held;
  await page.screenshot({ path: join(evidence, "loading.png") });
  await page.getByTestId("linear-disconnect").click();
  await page.getByTestId("linear-connect").waitFor();
  pendingSummary?.();
  await assert.rejects(access(join(userData, "linear-token.bin")), { code: "ENOENT" });
  assert.equal(await page.getByTestId("linear-issue").count(), 0);
  console.log("PASS Linear fetch error, disconnect and pending-result invalidation");
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("documents-toggle").filter({ hasText: "문서 1" }).waitFor();
  await page.getByTestId("documents-toggle").click();
  await page.getByTestId("documents-item").waitFor();
  assert.ok((await page.getByTestId("documents-group").innerText()).startsWith("Linear (1)"));
  assert.equal(await page.getByTestId("documents-item").getByRole("link").getAttribute("href"), issues[0].url);
  await page.screenshot({ path: join(evidence, "related-document.png") });
  console.log("PASS Linear issue appears in related documents without a change provider");
} finally {
  const exited = once(app.process(), "close", { signal: AbortSignal.timeout(15_000) });
  for (const window of app.windows()) await window.close();
  await exited;
  await gateway.close();
  api.closeAllConnections();
  await new Promise(resolve => api.close(resolve));
  await rm(userData, { recursive: true, force: true });
}
