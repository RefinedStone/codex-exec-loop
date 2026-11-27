import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdtemp, readFile, writeFile, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { _electron as electron } from "playwright-core";
import { APPROVAL_THREAD_LIST_RECORDING, INTERRUPTED_TURN_RECORDING, startFakeGateway, uniqueTestEndpoint } from "../fake-gateway/fakeGateway.ts";

const root = await mkdtemp(join(tmpdir(), "akra-attachment-"));
const desktop = join(dirname(fileURLToPath(import.meta.url)), "../..");
const run = promisify(execFile);
const original = "``` end \"quoted\" & <code>\r\n";
const file = join(root, "sample.txt");
const endpoint = uniqueTestEndpoint();
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_USER_DATA: join(root, "user-data"), AKRA_DESKTOP_HIDDEN: "1" };
delete env.ELECTRON_RUN_AS_NODE;
async function windowScenario(action) {
  const app = await electron.launch({ executablePath: createRequire(import.meta.url)("electron"), args: [desktop], env });
  try {
    const page = await app.firstWindow();
    await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor();
    await page.getByTestId("thread-entry").first().click();
    await page.getByTestId("conversation").waitFor();
    if (process.env.AKRA_QA_SCREENSHOT) await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].showInactive());
    await action(page);
  } finally { await app.close(); }
}

try {
  await run("git", ["init", "-q"], { cwd: root });
  await writeFile(file, `before\r\n${original}after\r\n`);
  await run("git", ["-c", "core.autocrlf=false", "add", "sample.txt"], { cwd: root });
  const recordings = [];
  for (const [i, source] of [APPROVAL_THREAD_LIST_RECORDING, INTERRUPTED_TURN_RECORDING].entries()) {
    const recording = join(root, `record-${i}.jsonl`);
    await writeFile(recording, (await readFile(source, "utf8")).replaceAll("/work/example-app", root.replaceAll("\\", "/")));
    recordings.push(recording);
  }
  const sent = [];
  const gateway = await startFakeGateway(endpoint, recordings, { appServer(method, params) {
    if (method === "akra/context/manifest") return { result: { manifest: { id: "fixture-manifest" }, changed: false } };
    if (method === "turn/start" || method === "turn/steer") {
      sent.push(params);
      return { result: { turn: { id: "received-turn" }, turnId: "received-turn" } };
    }
  } });
  try {
    await windowScenario(async page => {
      await page.getByTestId("composer-input").fill("review this");
      await page.getByText("파일·행 첨부", { exact: true }).click();
      await page.getByLabel("저장소 상대 경로").fill("sample.txt");
      await page.getByLabel("시작 줄").fill("2");
      await page.getByLabel("끝 줄").fill("2");
      await page.getByRole("button", { name: "선택 첨부" }).click();
      await page.getByText("보낼 정확한 내용 미리보기").click();
      const payload = JSON.parse(await page.getByTestId("file-message-preview").innerText());
      assert.equal(payload.quotedContext[0].selected, original);
      assert.equal(payload.quotedContext[0].sourceKind, "worktree_file");
      assert.equal(payload.quotedContext[0].side, "new");
      assert.equal(payload.quotedContext[0].startLine, 2);
      assert.equal(await page.getByTestId("composer-input").inputValue(), "review this");
    });
    await writeFile(file, "before\r\nchanged\r\nafter\r\n");
    await windowScenario(async page => {
      await page.getByTestId("composer-send").click();
      const consent = page.getByLabel("이전 판 기준으로 보내기");
      await consent.waitFor();
      assert.equal(sent.length, 0);
      await consent.check();
      await page.getByText("보낼 정확한 내용 미리보기").click();
      const preview = await page.getByTestId("file-message-preview").innerText();
      const payload = JSON.parse(preview);
      assert.equal(payload.quotedContext[0].selected, original);
      if (process.env.AKRA_QA_SCREENSHOT) await page.screenshot({ path: process.env.AKRA_QA_SCREENSHOT });
      await page.getByTestId("composer-send").click();
      await page.getByTestId("file-message-preview").waitFor({ state: "detached" });
      assert.equal(sent.length, 1);
      assert.equal(sent[0].threadId, payload.target.threadId);
      assert.equal(sent[0].input[0].text, preview);
    });
    console.log("PASS Electron capture, restart, stale consent, and byte-identical Gateway delivery");
  } finally { await gateway.close(); }
} finally { await rm(root, { recursive: true, force: true, maxRetries: 5 }); }
