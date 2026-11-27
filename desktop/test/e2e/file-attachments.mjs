import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { once } from "node:events";
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
async function windowScenario(action, gatewayEndpoint = endpoint) {
  const app = await electron.launch({ executablePath: process.env.AKRA_ELECTRON_EXECUTABLE ?? createRequire(import.meta.url)("electron"),
    args: [desktop], env: { ...env, AKRA_GATEWAY_ENDPOINT: gatewayEndpoint } });
  try {
    const page = await app.firstWindow();
    await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor();
    await page.getByTestId("thread-entry").first().click();
    await page.getByTestId("conversation").waitFor();
    if (process.env.AKRA_QA_SCREENSHOT) await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].showInactive());
    await action(page);
  } finally {
    const exited = once(app.process(), "exit", { signal: AbortSignal.timeout(10_000) });
    for (const win of app.windows()) await win.close();
    await exited;
  }
}

try {
  const git = args => run("git", ["-c", "core.autocrlf=false", ...args], { cwd: root, windowsHide: true,
    env: { ...Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith("GIT_"))), GIT_MASTER: "1" } });
  await git(["init", "-q"]);
  await writeFile(file, `before\r\n${original}after\r\n`);
  const oldPath = "deleted-lines.txt", newPath = "added-lines.txt";
  await writeFile(join(root, oldPath), "one\r\nINDEX_ONLY\r\nold tail\r\nend\r\n");
  await writeFile(join(root, newPath), "prefix\nend\n");
  await git(["add", "--", "sample.txt", oldPath, newPath]);
  await writeFile(join(root, oldPath), "one\r\nREPLACED\r\nchanged tail\r\nend\r\n");
  await writeFile(join(root, newPath), "prefix\nWORKTREE_ONLY\nnew tail\nend\n");
  const staged = (await git(["diff", "--cached"])).stdout;
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
      await page.getByTestId("review-toggle").click();
      await page.getByLabel("비교", { exact: true }).selectOption("index_worktree");
      for (const [path, side, code] of [[oldPath, "old", "INDEX_ONLY\r\nold tail\r\n"], [newPath, "new", "WORKTREE_ONLY\nnew tail\n"]]) {
        const entry = page.getByTestId("git-file").filter({ has: page.locator(`summary > code[title="${path}"]`) });
        await entry.locator("summary").first().click();
        await entry.getByLabel("선택 면").selectOption(side);
        await entry.getByLabel("선택 시작 행").fill("2");
        await entry.getByLabel("선택 끝 행").fill("3");
        await entry.getByRole("button", { name: "원문 범위 보기" }).click();
        await entry.getByRole("button", { name: "AI에게 피드백" }).click();
        await page.getByTestId("feedback-selection").waitFor();
        assert.equal(await page.getByTestId("feedback-selection").textContent(), code);
        await page.getByLabel("수정 지시").fill(`review ${side}`);
        await page.getByRole("button", { name: "지시에 담기" }).click();
      }
      assert.equal(sent.length, 0);
      const cards = page.getByTestId("file-attachment-card");
      await cards.filter({ hasText: newPath }).getByRole("button", { name: "위로" }).click();
      await cards.filter({ hasText: newPath }).getByLabel("피드백 수정").fill("check replacement");
      await cards.filter({ hasText: oldPath }).getByLabel("포함").uncheck();
      const excluded = JSON.parse(await page.getByTestId("file-message-preview").innerText());
      assert.deepEqual(excluded.quotedContext.map(item => item.filePath), ["sample.txt", newPath]);
      // Keep an explicitly excluded fourth attachment through submission of the other three.
      await page.getByLabel("시작 줄", { exact: true }).fill("1");
      await page.getByLabel("끝 줄", { exact: true }).fill("1");
      await page.getByRole("button", { name: "선택 첨부" }).click();
      await cards.filter({ hasText: "sample.txt:1–1" }).getByLabel("포함").uncheck();
    });
    const otherEndpoint = uniqueTestEndpoint();
    const other = await startFakeGateway(otherEndpoint, recordings);
    try {
      await windowScenario(async page => {
        assert.equal(await page.getByTestId("file-attachment-card").count(), 0);
        assert.equal(await page.getByTestId("composer-input").inputValue(), "");
      }, otherEndpoint);
    } finally { await other.close(); }
    await writeFile(file, "before\r\nchanged\r\nafter\r\n");
    await writeFile(join(root, oldPath), "later\n");
    await windowScenario(async page => {
      const cards = page.getByTestId("file-attachment-card");
      assert.equal(await cards.filter({ hasText: oldPath }).getByLabel("포함").isChecked(), false);
      assert.equal(await page.getByTestId("composer-input").inputValue(), "review this");
      await cards.filter({ hasText: oldPath }).getByLabel("포함").check();
      await page.getByTestId("composer-send").click();
      const consent = cards.filter({ hasText: "sample.txt:2–2" }).getByLabel("이전 판 기준으로 보내기");
      await consent.waitFor();
      assert.equal(sent.length, 0);
      await consent.check();
      await cards.filter({ hasText: oldPath }).getByLabel("이전 판 기준으로 보내기").check();
      await page.getByText("보낼 정확한 내용 미리보기").click();
      const preview = await page.getByTestId("file-message-preview").innerText();
      const payload = JSON.parse(preview);
      assert.equal(payload.quotedContext[0].selected, original);
      assert.deepEqual(payload.quotedContext.slice(1).map(({ filePath, sourceKind, side, startLine, endLine, selected, feedback, useOriginal }) =>
        [filePath, sourceKind, side, startLine, endLine, selected, feedback, useOriginal]), [
        [newPath, "index_worktree", "new", 2, 3, "WORKTREE_ONLY\nnew tail\n", "check replacement", false],
        [oldPath, "index_worktree", "old", 2, 3, "INDEX_ONLY\r\nold tail\r\n", "review old", true],
      ]);
      assert.deepEqual(payload.quotedContext.slice(1).map(i => i.diffExcerpt), ["+WORKTREE_ONLY\n+new tail", "-INDEX_ONLY\n-old tail"]);
      if (process.env.AKRA_QA_SCREENSHOT) await page.screenshot({ path: process.env.AKRA_QA_SCREENSHOT });
      await page.getByTestId("composer-send").click();
      await page.getByTestId("file-message-preview").waitFor({ state: "detached" });
      assert.equal(sent.length, 1);
      assert.equal(sent[0].threadId, payload.target.threadId);
      assert.equal(sent[0].input[0].text, preview);
      assert.equal(await cards.count(), 1);
      assert.equal(await cards.filter({ hasText: "sample.txt:1–1" }).getByLabel("포함").isChecked(), false);
      assert.equal((await git(["diff", "--cached"])).stdout, staged);
    });
    console.log("PASS Electron scoped restart, explicit exclusion, stale consent, INDEX_ONLY/WORKTREE_ONLY ranges and byte-identical Gateway delivery");
  } finally { await gateway.close(); }
} finally { await rm(root, { recursive: true, force: true, maxRetries: 5 }); }
