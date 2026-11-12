// 실제 git 과 Electron IPC 를 사용한다. Gateway 대화와 확인 창의 답만 시험에서 제공한다.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "../..");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);
const root = await mkdtemp(join(tmpdir(), "akra-hunk-ui-"));
const run = promisify(execFile);
const git = async (...args) => (await run("git", ["-c", "user.name=Tester", "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false", ...args], { cwd: root, windowsHide: true })).stdout;
await git("init", "-q", "-b", "main");
await git("config", "core.autocrlf", "false");
const lines = Array.from({ length: 45 }, (_, index) => `line ${index + 1}`);
const file = join(root, "example.txt");
await writeFile(file, `${lines.join("\n")}\n`);
await git("add", "example.txt");
await git("commit", "-q", "-m", "Initial");
lines[1] = "already staged";
await writeFile(file, `${lines.join("\n")}\n`);
await git("add", "example.txt");
lines[15] = "stage this hunk";
lines[35] = "discard this hunk";
await writeFile(file, `${lines.join("\n")}\n`);

const recordings = [];
for (const [index, source] of [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING].entries()) {
  const target = join(root, `recording-${index}.jsonl`);
  await writeFile(target, (await readFile(source, "utf8")).replaceAll("/work/example-app", root.replaceAll("\\", "/")));
  recordings.push(target);
}
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, recordings);
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: join(root, "user-data") };
delete env.ELECTRON_RUN_AS_NODE;
const app = await electron.launch({ executablePath: createRequire(import.meta.url)("electron"), args: [desktopDir], env });
try {
  await app.evaluate(({ dialog }) => {
    let confirmations = 0;
    dialog.showMessageBox = async () => ({ response: confirmations++ === 0 ? 1 : 0, checkboxChecked: false });
  });
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("review-toggle").click();
  const entry = page.getByTestId("git-file").filter({ hasText: "example.txt" });
  await entry.locator("summary").first().click();
  await page.getByTestId("git-hunk-stage").first().waitFor();
  assert.equal(await page.getByTestId("git-hunk").count(), 2);
  if (process.env.AKRA_QA_SCREENSHOT) {
    await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].showInactive());
    await page.screenshot({ path: process.env.AKRA_QA_SCREENSHOT, animations: "disabled" });
    await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1000, 800));
    await page.screenshot({ path: process.env.AKRA_QA_SCREENSHOT.replace(".png", "-compact.png"), animations: "disabled" });
    await page.getByTitle("밝은 화면 / 어두운 화면", { exact: true }).click();
    await page.screenshot({ path: process.env.AKRA_QA_SCREENSHOT.replace(".png", "-compact-dark.png"), animations: "disabled" });
    await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1200, 800));
    await page.screenshot({ path: process.env.AKRA_QA_SCREENSHOT.replace(".png", "-dark.png"), animations: "disabled" });
  }
  await page.getByTestId("git-hunk-stage").first().click();
  await page.getByTestId("git-hunks").waitFor({ state: "detached" });
  const staged = await git("diff", "--cached");
  assert.ok(staged.includes("+already staged"));
  assert.ok(staged.includes("+stage this hunk"));
  assert.ok(!staged.includes("+discard this hunk"));
  await entry.locator("summary").first().click();
  await page.getByTestId("git-hunk-discard").waitFor();
  assert.equal(await page.getByTestId("git-hunk").count(), 1);
  // 클릭 전에 비활성화와 재활성화를 구독한다. 재활성화가 취소 응답의 완료이다.
  const cancellationFinished = page.getByTestId("git-hunk-discard").evaluate((button) => {
    return new Promise((resolve, reject) => {
      const signal = AbortSignal.timeout(5000);
      let wasDisabled = false;
      const observer = new MutationObserver(() => {
        wasDisabled ||= button.disabled;
        if (wasDisabled && !button.disabled) {
          observer.disconnect();
          resolve();
        }
      });
      signal.addEventListener("abort", () => { observer.disconnect(); reject(signal.reason); }, { once: true });
      observer.observe(button, { attributes: true, attributeFilter: ["disabled"] });
    });
  });
  await page.getByTestId("git-hunk-discard").click();
  await cancellationFinished;
  assert.ok((await readFile(file, "utf8")).includes("discard this hunk"));
  await page.getByTestId("git-hunk-discard").click();
  await page.getByTestId("git-hunks").waitFor({ state: "detached" });
  assert.equal(await git("diff", "--", "example.txt"), "");
  assert.equal(await git("diff", "--cached"), staged);
  console.log("PASS hunk stage preserves earlier staged content; cancel keeps the hunk; discard changes only the chosen unstaged hunk");
} finally {
  const exited = once(app.process(), "exit", { signal: AbortSignal.timeout(10_000) });
  for (const window of app.windows()) await window.close();
  await exited;
  await gateway.close();
  await rm(root, { recursive: true, force: true });
}
