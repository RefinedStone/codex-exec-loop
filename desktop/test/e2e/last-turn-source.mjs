// Actual Desktop transcript reduction and rendering; only the Gateway transport is a replay.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { _electron as electron } from "playwright-core";

const desktop = join(dirname(fileURLToPath(import.meta.url)), "../..");
const fake = await import(pathToFileURL(join(desktop, "test/fake-gateway/fakeGateway.ts")).href);
const root = await mkdtemp(join(tmpdir(), "akra-last-turn-"));
const evidence = resolve(process.env.AKRA_TURN_EVIDENCE ?? join(desktop, "test-results", "last-turn-source"));
await mkdir(evidence, { recursive: true });
const run = promisify(execFile);
const gitEnv = { ...Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("GIT_"))), GIT_MASTER: "1" };
const git = async (...args) => (await run("git", ["-c", "user.name=Tester", "-c", "user.email=fixture@localhost.invalid",
  "-c", "commit.gpgsign=false", ...args], { cwd: root, windowsHide: true, env: gitEnv })).stdout;
await git("init", "-q", "-b", "main"); await git("config", "core.autocrlf", "false");
await writeFile(join(root, "file.txt"), "HEAD_ONLY\n");
await git("add", "file.txt"); await git("commit", "-qm", "unrelated HEAD");
const indexBefore = await git("diff", "--cached");
const threadList = join(root, "thread-list.jsonl");
await writeFile(threadList, (await readFile(fake.APPROVAL_THREAD_LIST_RECORDING, "utf8")).replaceAll("/work/example-app", root.replaceAll("\\", "/")));
const recording = (await readFile(fake.INTERRUPTED_TURN_RECORDING, "utf8")).trim().split("\n").map(line => {
  const row = JSON.parse(line);
  return { direction: row.direction, message: JSON.parse(row.text) };
});
const resume = recording.find(row => row.message.result?.thread)?.message.result;
assert.ok(resume);
const patch = prefix => `@@ -10,2 +20,2 @@\n-${prefix}_OLD\n+${prefix}_NEW\n ${prefix}_SHARED\n`;
const results = [];
try {
  for (const mode of ["items", "aggregate"]) {
    const response = structuredClone(resume);
    response.cwd = root; response.thread.cwd = root;
    response.thread.turns = [response.thread.turns.at(-1)];
    const turn = response.thread.turns[0];
    turn.status = mode === "items" ? "inProgress" : "completed";
    turn.items = [{ type: "fileChange", id: "received-file-change", status: "completed",
      changes: [{ path: "file.txt", kind: { type: "update", move_path: null }, diff: patch("ITEM") }] },
      { type: "agentMessage", id: "streaming-reply", text: "Recorded reply" }];
    const rows = [
      recording[0], recording[1], recording[2], recording[3],
      { direction: "gateway", message: { jsonrpc: "2.0", id: 2, result: response } },
      ...(mode === "aggregate" ? [{ direction: "gateway", message: {
        method: "turn/diff/updated", params: { threadId: response.thread.id, turnId: turn.id,
          diff: "diff --git a/file.txt b/file.txt\n--- a/file.txt\n+++ b/file.txt\n" + patch("AGGREGATE") },
      } }] : []),
    ];
    const replay = join(root, `${mode}.jsonl`);
    await writeFile(replay, rows.map(row => JSON.stringify({ direction: row.direction, text: JSON.stringify(row.message) })).join("\n") + "\n");
    const endpoint = fake.uniqueTestEndpoint();
    const gateway = await fake.startFakeGateway(endpoint, [threadList, replay]);
    const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: join(root, `profile-${mode}`) };
    delete env.ELECTRON_RUN_AS_NODE;
    let snapshotId;
    const prefix = mode === "items" ? "ITEM" : "AGGREGATE";
    try {
      for (const pass of ["initial", "reopen"]) {
        const app = await electron.launch({ executablePath: process.env.AKRA_ELECTRON_EXECUTABLE ?? createRequire(import.meta.url)("electron"),
          args: [desktop], env });
        try {
          const page = await app.firstWindow();
          await app.evaluate(({ BrowserWindow }) => { const window = BrowserWindow.getAllWindows()[0]; window.setSize(1280, 900); window.showInactive(); });
          await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
          await page.getByTestId("thread-entry").first().click();
          const changes = page.getByTestId("turn-changes").last();
          await changes.getByTestId("diff-file").locator("summary").first().click();
          const source = changes.getByTestId("turn-source");
          await source.getByRole("button", { name: "턴 원문 범위 읽기" }).waitFor();
          const id = await source.getByTestId("turn-source-id").textContent();
          assert.match(id, /^[a-f0-9]{64}$/);
          if (snapshotId) assert.equal(id, snapshotId, "Reopening must use the same supplied turn data, not new disk content.");
          snapshotId = id;
          await source.getByLabel("턴 원문 면").selectOption("old");
          await source.getByLabel("턴 원문 시작 행").fill("10");
          await source.getByLabel("턴 원문 끝 행").fill("11");
          await source.getByRole("button", { name: "턴 원문 범위 읽기" }).click();
          assert.equal(await source.getByTestId("turn-source-excerpt").textContent(), `${prefix}_OLD\n${prefix}_SHARED\n`);
          await writeFile(join(root, "file.txt"), `${mode}_${pass}_LATER_DISK\n`);
          if (mode === "items") {
            gateway.sendAppServerNotification("item/agentMessage/delta", {
              threadId: response.thread.id, turnId: turn.id, itemId: "streaming-reply", delta: " with one new delta",
            });
            await page.getByTestId("item-agent").getByText("Recorded reply with one new delta").waitFor();
            assert.equal(await source.getByTestId("turn-source-excerpt").count(), 1, "A streamed reply must not clear the visible recorded range.");
            assert.equal(await source.getByTestId("turn-source-excerpt").textContent(), `${prefix}_OLD\n${prefix}_SHARED\n`);
            assert.equal(await source.getByTestId("turn-source-id").textContent(), snapshotId);
            assert.equal(await source.getByLabel("턴 원문 면").inputValue(), "old");
            assert.equal(await source.getByLabel("턴 원문 시작 행").inputValue(), "10");
            assert.equal(await source.getByLabel("턴 원문 끝 행").inputValue(), "11");
          }
          await source.getByRole("button", { name: "턴 원문 범위 읽기" }).click();
          assert.equal(await source.getByTestId("turn-source-excerpt").textContent(), `${prefix}_OLD\n${prefix}_SHARED\n`);
          await source.getByLabel("턴 원문 면").selectOption("new");
          await source.getByLabel("턴 원문 시작 행").fill("20");
          await source.getByLabel("턴 원문 끝 행").fill("21");
          await source.getByRole("button", { name: "턴 원문 범위 읽기" }).click();
          assert.equal(await source.getByTestId("turn-source-excerpt").textContent(), `${prefix}_NEW\n${prefix}_SHARED\n`);
          await page.screenshot({ path: join(evidence, `${mode}-${pass}-excerpt.png`), animations: "disabled" });
          await source.getByLabel("턴 원문 시작 행").fill("19");
          await source.getByRole("button", { name: "턴 원문 범위 읽기" }).click();
          await source.getByRole("status").filter({ hasText: "unavailable: context_not_provided" }).waitFor();
          assert.equal(await source.getByTestId("turn-source-excerpt").count(), 0);
          await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1000, 800));
          await page.screenshot({ path: join(evidence, `${mode}-${pass}-unavailable.png`), animations: "disabled" });
        } finally {
          const exited = once(app.process(), "exit", { signal: AbortSignal.timeout(10_000) });
          for (const window of app.windows()) await window.close();
          await exited;
        }
      }
      assert.equal(gateway.receivedMethods.filter(method => method === "turn/start" || method === "turn/steer").length, 0);
      results.push({ mode, snapshotId, modelTurns: 0, result: "passed" });
    } finally { await gateway.close(); }
  }
  assert.equal(await git("diff", "--cached"), indexBefore);
  await writeFile(join(evidence, "receipt.json"), JSON.stringify({ results, indexUnchanged: true }, null, 2));
  console.log("PASS last-turn item and aggregate excerpts survive disk edits and Desktop restart; missing context is unavailable; zero model turns");
} finally {
  await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 });
}
