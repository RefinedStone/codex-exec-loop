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
    await app.close();
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
  const sampleId = "a".repeat(64), oldId = "b".repeat(64), newId = "c".repeat(64), changedId = "d".repeat(64);
  let changedLedger = false, dropAcknowledgment = false;
  const gateway = await startFakeGateway(endpoint, recordings, { appServer(method, params, requestId) {
    if (method === "akra/context/manifest") {
      const path = params.paths[0];
      const changed = changedLedger && path === "sample.txt";
      const id = path === oldPath ? oldId : path === newPath ? newId : changed ? changedId : sampleId;
      return { result: { manifest: { id, entries: [
        { kind: "github", key: "fixture#1", selected: true, reason: "path", observedVersion: changed ? "v2" : null },
        { kind: "github", key: "fixture#2", selected: false, reason: "explicit_exclude", observedVersion: "v1" },
      ] }, changed: params.previousId !== undefined && params.previousId !== id, unverifiedCount: changed ? 0 : 1 } };
    }
    if (method === "turn/start" || method === "turn/steer") {
      sent.push({ params, requestId });
      if (dropAcknowledgment) {
        gateway.disconnectClients();
        return null;
      }
      return { result: { turn: { id: "received-turn" }, turnId: params.expectedTurnId } };
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
    changedLedger = true;
    await windowScenario(async page => {
      const cards = page.getByTestId("file-attachment-card");
      const oldCard = cards.filter({ hasText: oldPath });
      assert.equal(await oldCard.getByLabel("포함").isChecked(), false);
      await oldCard.getByText("요청에서 제외").waitFor();
      assert.equal(await page.getByTestId("composer-input").inputValue(), "review this");
      await oldCard.getByLabel("포함").check();
      await page.getByTestId("composer-send").click();
      const sampleCard = cards.filter({ hasText: "sample.txt:2–2" });
      const consent = sampleCard.getByLabel("이전 판 기준으로 보내기");
      await consent.waitFor();
      const originalLedger = sampleCard.getByTestId("attachment-ledger").filter({ hasText: "버전 미확인 1" });
      await originalLedger.locator("summary").click();
      await originalLedger.getByText("fixture#2 · 제외 (explicit_exclude) · v1", { exact: false }).waitFor();
      const currentLedger = sampleCard.getByTestId("attachment-ledger").filter({ hasText: "버전 미확인 0" });
      await currentLedger.locator("summary").click();
      await currentLedger.getByText("fixture#1 · 선택 (path) · v2", { exact: false }).waitFor();
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
      assert.equal(typeof sent[0].params.threadId, "string");
      assert.equal(Object.hasOwn(payload, "target"), false);
      for (const item of payload.quotedContext) {
        for (const key of ["origin", "repositoryId", "worktreeId", "snapshotId", "headOid", "fingerprint", "staleReason"]) {
          assert.equal(Object.hasOwn(item, key), false, key);
        }
      }
      assert.equal(sent[0].params.input[0].text, preview);
      assert.deepEqual(payload.quotedContext.map(item => item.manifestId), [sampleId, newId, oldId]);
      assert.equal(preview.includes("fixture#"), false);
      assert.equal(await cards.count(), 1);
      assert.equal(await cards.filter({ hasText: "sample.txt:1–1" }).getByLabel("포함").isChecked(), false);
      assert.equal((await git(["diff", "--cached"])).stdout, staged);
    });
    await windowScenario(async page => {
      const acknowledgment = page.getByTestId("attachment-receipt").and(page.locator('[data-status="acknowledged"]'));
      await acknowledgment.locator("summary").click();
      assert.deepEqual(await acknowledgment.locator("code").allTextContents(),
        [String(sent[0].requestId), sent[0].params.expectedTurnId ?? "received-turn", sampleId, newId, oldId]);
      assert.equal(sent.length, 1);

      const excluded = page.getByTestId("file-attachment-card").filter({ hasText: "sample.txt:1–1" });
      await excluded.getByLabel("포함").check();
      await page.getByTestId("composer-input").fill("do not replay this");
      await page.getByTestId("composer-send").click();
      await excluded.getByLabel("이전 판 기준으로 보내기").check();
      dropAcknowledgment = true;
      await page.getByTestId("composer-send").click();
      await page.locator(".composer-area .error").filter({ hasText: /turn\/(start|steer): / }).waitFor();
      await page.getByTestId("attachment-receipt").and(page.locator('[data-status="unknown"]')).waitFor();
      assert.equal(sent.length, 2);
      assert.deepEqual(JSON.parse(sent[1].params.input[0].text).quotedContext.map(item => item.manifestId), [sampleId]);
    });
    await windowScenario(async page => {
      const receipts = page.getByTestId("attachment-receipt");
      const unknown = receipts.and(page.locator('[data-status="unknown"]'));
      await unknown.locator("summary").click();
      assert.deepEqual(await unknown.locator("code").allTextContents(), [String(sent[1].requestId), sampleId]);
      assert.equal(await receipts.count(), 2);
      assert.equal(await page.getByTestId("composer-input").inputValue(), "do not replay this");
      const stored = await page.evaluate(() => Object.keys(localStorage).filter(key => key.startsWith("akra.attachment-receipt.v1:"))
        .map(key => JSON.parse(localStorage.getItem(key))));
      assert.equal(stored.length, 2);
      for (const receipt of stored) assert.deepEqual(Object.keys(receipt).sort(), ["manifestIds", "requestId", "scope", "turnId"]);
      assert.equal(sent.length, 2);
    });
    console.log("PASS Electron scoped restart, stale/excluded/unverified ledger, byte-identical payload, ack and unknown receipts, no replay");
  } finally { await gateway.close(); }
} finally { await rm(root, { recursive: true, force: true, maxRetries: 5 }); }
