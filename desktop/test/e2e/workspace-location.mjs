// #2833: render two project locations from the same synthetic repository.
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { _electron as electron } from "playwright-core";
import { BASIC_RECORDING, startFakeGateway, uniqueTestEndpoint } from "../fake-gateway/fakeGateway.ts";

const desktopDir = resolve(fileURLToPath(new URL("../../", import.meta.url)));
const userData = await mkdtemp(`${tmpdir()}/akra-workspace-location-`);
const endpoint = uniqueTestEndpoint();
const recording = (await readFile(new URL("../fake-gateway/fixtures/thread-start.jsonl", import.meta.url), "utf8"))
  .trim().split(/\r?\n/).map(line => JSON.parse(JSON.parse(line).text));
const template = recording.find(frame => frame.result?.thread).result.thread;
const threads = ["one", "two"].map(suffix => ({
  ...template,
  id: `workspace-${suffix}`, name: `Workspace ${suffix}`, cwd: `/fixture/repo/worktrees/${suffix}`,
  gitInfo: { branch: `akra-conversation/${suffix}`, sha: null, originUrl: null },
}));
const gateway = await startFakeGateway(endpoint, [BASIC_RECORDING], {
  appServer(method, params) {
    if (method === "thread/list") return { result: { data: threads, nextCursor: null } };
    if (method === "thread/resume") return { result: { thread: threads.find(thread => thread.id === params.threadId) } };
    if (method === "thread/turns/list") return { result: { data: [], nextCursor: null } };
    if (method.startsWith("turn/")) return { error: { code: -32601, message: "No model turns in this fixture" } };
    return undefined;
  },
});
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;
let app;
try {
  app = await electron.launch({ executablePath: createRequire(import.meta.url)("electron"), args: [desktopDir], env, timeout: 30_000 });
  const page = await app.firstWindow({ timeout: 30_000 });
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  for (const thread of threads) {
    const header = page.locator(`.project-header[title="${thread.cwd}"]`);
    const row = header.locator("..").getByTestId("thread-entry").filter({ hasText: thread.name });
    await row.getByText(thread.gitInfo.branch, { exact: true }).waitFor({ timeout: 30_000 });
    assert.equal(await row.count(), 1);
  }
  assert(!gateway.receivedMethods.some(method => method.startsWith("turn/")));
  console.log("PASS Electron sidebar displays both exact worktree paths and branches without sending model turns");
} finally {
  try {
    await app?.close();
  } finally {
    await gateway.close();
    await rm(userData, { recursive: true, force: true, maxRetries: 5 });
  }
}
