import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { cpus, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { _electron as electron } from "playwright-core";

const desktop = join(dirname(fileURLToPath(import.meta.url)), "../..");
const fake = await import(pathToFileURL(join(desktop, "test/fake-gateway/fakeGateway.ts")).href);
const root = await mkdtemp(join(tmpdir(), "akra-diff-navigation-"));
const evidence = resolve(process.env.AKRA_DIFF_EVIDENCE ?? join(desktop, "test-results", "diff-navigation"));
await mkdir(evidence, { recursive: true });
const run = promisify(execFile);
const gitEnv = { ...Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("GIT_"))), GIT_MASTER: "1" };
const git = async (...args) => (await run("git", ["-c", "user.name=Tester", "-c", "user.email=fixture@localhost.invalid",
  "-c", "commit.gpgsign=false", ...args], { cwd: root, windowsHide: true, env: gitEnv })).stdout;
await git("init", "-q", "-b", "main"); await git("config", "core.autocrlf", "false");
await writeFile(join(root, ".git", "info", "exclude"), "recording-*.jsonl\nprofile/\n");
const files = Array.from({ length: 98 }, (_, index) => ({ path: `small-${String(index).padStart(3, "0")}.ts`, count: 40 }));
files.push({ path: "large.ts", count: 1000 }, { path: "context.ts", count: 400 });
const content = (file, changed) => Array.from({ length: file.count }, (_, index) =>
  file.path === "context.ts" ? (changed && index % 5 === 4 ? `const WORKTREE_${index + 1} = true;` : `const unchanged_${index + 1} = ${index + 1};`)
    : `const ${changed ? "updated" : "original"}_${index + 1} = ${index + 1};`).join("\n") + "\n";
for (const file of files) await writeFile(join(root, file.path), content(file, false));
await git("add", "."); await git("commit", "-qm", "100 file fixture");
const context = files.find(file => file.path === "context.ts");
assert.ok(context);
await writeFile(join(root, context.path), content(context, false).replace("const unchanged_40 = 40;", "const INDEX_ONLY = true;"));
await git("add", "context.ts");
for (const file of files) await writeFile(join(root, file.path), content(file, true));
const counts = (await git("diff", "--numstat")).trim().split("\n").map(line => line.split("\t"));
assert.equal(counts.length, 100);
assert.equal(counts.reduce((sum, row) => sum + Number(row[0]) + Number(row[1]), 0), 10000);
const indexBefore = await git("diff", "--cached");
const recordings = [];
for (const [index, source] of [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING].entries()) {
  const recording = join(root, `recording-${index}.jsonl`);
  await writeFile(recording, (await readFile(source, "utf8")).replaceAll("/work/example-app", root.replaceAll("\\", "/")));
  recordings.push(recording);
}
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, recordings);
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: join(root, "profile") };
delete env.ELECTRON_RUN_AS_NODE;
const app = await electron.launch({ executablePath: process.env.AKRA_ELECTRON_EXECUTABLE ?? createRequire(import.meta.url)("electron"),
  args: [desktop], env });
try {
  const page = await app.firstWindow();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].showInactive());
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  console.log("STEP connected");
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("review-toggle").click();
  console.log("STEP review");
  await page.getByLabel("비교", { exact: true }).selectOption("index_worktree");
  const fileEntry = path => page.getByTestId("git-file").filter({ has: page.locator(`summary > code[title="${path}"]`) });
  const entry = fileEntry("context.ts");
  await entry.locator("summary").first().click();
  const view = entry.getByTestId("snapshot-diff");
  await view.locator('[data-deletions] [data-column-number="40"]').click();
  await view.locator('[data-deletions] [data-column-number="41"]').click({ modifiers: ["Shift"] });
  assert.equal(await view.getByLabel("선택 시작 행").inputValue(), "40");
  assert.equal(await view.getByLabel("선택 끝 행").inputValue(), "41");
  await view.locator('[data-deletions] [data-column-number="41"]').hover();
  await page.mouse.down();
  await view.locator('[data-deletions] [data-column-number="43"]').hover();
  await page.evaluate(() => new Promise(resolveFrame => requestAnimationFrame(resolveFrame)));
  await page.mouse.up();
  await page.waitForFunction(() => document.querySelector('input[aria-label="선택 끝 행"]')?.value === "43");
  assert.equal(await view.getByLabel("선택 면").inputValue(), "old");
  assert.equal(await view.getByLabel("선택 끝 행").inputValue(), "43");
  await view.getByLabel("선택 면").selectOption("old");
  await view.getByLabel("읽은 파일 변경 내용 찾기").fill("INDEX_ONLY");
  await view.getByRole("button", { name: "다음 결과", exact: true }).click();
  const snippet = view.getByTestId("snapshot-context");
  await snippet.locator('[data-context-line="40"] code').filter({ hasText: "INDEX_ONLY" }).waitFor();
  console.log("STEP old-context");
  await snippet.getByRole("button", { name: "앞뒤 10행 더 보기" }).click();
  await snippet.locator('[data-context-line="20"] code').filter({ hasText: "const unchanged_20 = 20;" }).waitFor();
  await page.getByTestId("diff-style-toggle").click();
  await view.locator('[data-diff-style="unified"]').waitFor();
  assert.equal(await snippet.locator('[data-context-line="40"]').getAttribute("data-selected"), "true");
  await view.getByLabel("줄바꿈", { exact: true }).check();
  await view.getByLabel("원문 공백 표시").check();
  assert.ok((await snippet.locator('[data-context-line="40"] code').textContent()).includes("INDEX_ONLY"));
  await view.getByLabel("원문 공백 표시").uncheck();
  await view.getByRole("button", { name: "다음 hunk", exact: true }).click();
  await view.getByLabel("선택 시작 행").fill("40");
  await view.getByLabel("선택 끝 행").fill("40");
  await view.getByRole("button", { name: "원문 범위 보기" }).press("Enter");
  await snippet.locator('[data-context-line="40"] code').filter({ hasText: "INDEX_ONLY" }).waitFor();
  await app.evaluate(({ BrowserWindow }) => { BrowserWindow.getAllWindows()[0].setSize(3840, 2160); BrowserWindow.getAllWindows()[0].showInactive(); });
  await page.screenshot({ path: join(evidence, "context-4k.png"), animations: "disabled" });
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].webContents.setZoomFactor(1.5));
  await page.waitForFunction(() => document.querySelector('[data-testid="right-panel"]').getBoundingClientRect().right <= window.innerWidth + 1);
  await snippet.locator('[data-context-line="40"]').scrollIntoViewIfNeeded();
  const enlarged = await app.evaluate(async ({ BrowserWindow }) =>
    (await BrowserWindow.getAllWindows()[0].webContents.capturePage()).toPNG().toString("base64"));
  await writeFile(join(evidence, "context-4k-150.png"), Buffer.from(enlarged, "base64"));
  await app.evaluate(({ BrowserWindow }) => { const window = BrowserWindow.getAllWindows()[0]; window.webContents.setZoomFactor(1); window.setSize(1000, 800); });
  await page.screenshot({ path: join(evidence, "context-narrow.png"), animations: "disabled" });
  const sidebarWidth = (await page.getByTestId("folders").boundingBox()).width;
  const tabsHeight = (await page.locator(".right-panel > header").boundingBox()).height;
  await page.evaluate(() => { window.akra.zoom.step("panel:review", "in"); window.akra.zoom.step("panel:review", "in"); window.akra.zoom.step("panel:review", "in"); });
  await page.waitForFunction(() => getComputedStyle(document.querySelector('[data-zoom-scope="panel:review"]')).zoom === "1.5");
  assert.equal((await page.getByTestId("folders").boundingBox()).width, sidebarWidth);
  assert.equal((await page.locator(".right-panel > header").boundingBox()).height, tabsHeight);
  await snippet.locator('[data-context-line="40"]').scrollIntoViewIfNeeded();
  await page.screenshot({ path: join(evidence, "context-panel-150.png"), animations: "disabled" });
  await page.evaluate(() => window.akra.zoom.step("panel:review", "reset"));
  console.log("STEP screenshots");
  await writeFile(join(root, "context.ts"), content(context, true).replace("WORKTREE_40", "LATER_FILE"));
  await snippet.getByRole("button", { name: "앞뒤 10행 더 보기" }).click();
  await snippet.getByRole("status").filter({ hasText: "선택 후 변경됨" }).waitFor();
  assert.ok((await snippet.locator('[data-context-line="40"] code').textContent()).includes("INDEX_ONLY"));
  // Switching comparison removes the old selection and its in-flight context.
  await page.getByLabel("비교", { exact: true }).selectOption("head_index");
  await entry.getByTestId("git-diff-snapshot").filter({ hasText: "HEAD ↔ 스테이지" }).waitFor();
  assert.equal(await entry.getByTestId("snapshot-context").count(), 0);
  await page.getByLabel("비교", { exact: true }).selectOption("index_worktree");
  const large = fileEntry("large.ts");
  const started = performance.now();
  await large.locator("summary").first().click();
  const firstDisplay = large.getByText("const updated_1000 = 1000;", { exact: true }).waitFor().then(() => performance.now() - started);
  const composer = page.getByTestId("composer-input");
  const responseMs = [];
  for (let index = 0; index < 20; index += 1) {
    const tick = performance.now();
    await composer.fill(`typing while diff loads ${index}`);
    responseMs.push(performance.now() - tick);
  }
  await large.getByTestId("snapshot-diff").locator('[data-diff-style]').waitFor();
  const warm2000Ms = await firstDisplay;
  assert.equal(await composer.inputValue(), "typing while diff loads 19");
  assert.equal(await git("diff", "--cached"), indexBefore);
  await large.locator("summary").first().scrollIntoViewIfNeeded();
  await page.screenshot({ path: join(evidence, "large-diff.png"), animations: "disabled" });
  const switchMs = [];
  const switchCallMs = [];
  for (let index = 0; index < 20; index += 1) {
    const next = page.getByRole("button", { name: "다음 파일", exact: true });
    await next.evaluate(button => {
      button.dataset.measureNext = "true";
      delete button.dataset.frameMs;
      button.addEventListener("click", () => {
        const started = performance.now();
        requestAnimationFrame(() => requestAnimationFrame(() => {
          button.dataset.openCount = String(document.querySelectorAll('[data-testid="git-file"][open]').length);
          button.dataset.frameMs = String(performance.now() - started);
        }));
      }, { once: true });
    });
    const tick = performance.now();
    await next.press("Enter");
    switchCallMs.push(performance.now() - tick);
    await page.waitForFunction(() => document.querySelector('[data-measure-next]')?.hasAttribute("data-frame-ms"));
    const sample = await next.evaluate(button => ({ ms: Number(button.dataset.frameMs), open: Number(button.dataset.openCount) }));
    assert.equal(sample.open, 1, "File navigation must keep only the target file expanded.");
    switchMs.push(sample.ms);
  }
  const report = { platform: process.platform, cpu: cpus()[0]?.model, electron: await app.evaluate(() => process.versions.electron),
    files: 100, changedLines: 10000, largeChangedLines: 2000, warm2000Ms,
    inputP95Ms: responseMs.toSorted((a, b) => a - b)[18], fileSwitchP95Ms: switchMs.toSorted((a, b) => a - b)[18],
    fileSwitchAutomationP95Ms: switchCallMs.toSorted((a, b) => a - b)[18], responseMs, switchMs, switchCallMs,
    note: "Warm syntax engine; file capture is fresh. Input timings include Playwright IPC. File switch measures keyboard activation click through two animation frames. 150% uses Electron zoom, not an OS DPI change." };
  await writeFile(join(evidence, "timings.json"), JSON.stringify(report, null, 2));
  await writeFile(join(root, "fallback.txt"), Array.from({ length: 6000 }, (_, index) => `fallback ${index + 1}`).join("\n"));
  await writeFile(join(root, "oversized.txt"), "x".repeat((1 << 20) + 1));
  await writeFile(join(root, "binary.bin"), "binary\0bytes");
  await page.getByRole("button", { name: "새로 고침", exact: true }).click();
  const fallback = fileEntry("fallback.txt");
  await fallback.locator("summary").first().click();
  await fallback.getByRole("status").filter({ hasText: "면과 행 번호를 사용하십시오" }).waitFor();
  await fallback.getByRole("button", { name: "다음 표시 구간", exact: true }).waitFor();
  assert.equal(await fallback.locator(".diff-line").count(), 1000);
  await fallback.getByRole("button", { name: "다음 표시 구간", exact: true }).click();
  assert.equal(await fallback.locator(".diff-line").count(), 1000);
  await fallback.getByLabel("선택 시작 행").fill("");
  assert.equal(await fallback.getByRole("button", { name: "원문 범위 보기" }).isEnabled(), false);
  await fallback.getByLabel("선택 시작 행").fill("6000");
  await fallback.getByLabel("선택 끝 행").fill("6000");
  await fallback.getByRole("button", { name: "원문 범위 보기" }).click();
  await fallback.locator('[data-context-line="6000"] code').filter({ hasText: "fallback 6000" }).waitFor();
  await fallback.locator('[data-context-line="6000"]').scrollIntoViewIfNeeded();
  await page.screenshot({ path: join(evidence, "bounded-fallback.png"), animations: "disabled" });
  const oversized = fileEntry("oversized.txt");
  await oversized.locator("summary").first().click();
  await oversized.getByRole("status").filter({ hasText: "파일 크기 제한" }).waitFor();
  assert.ok(await page.locator('[data-zoom-scope="panel:review"]').evaluate(element =>
    element.scrollWidth <= element.clientWidth + 1), "Oversized source must scroll inside its code view, not the whole panel.");
  const binary = fileEntry("binary.bin");
  await binary.locator("summary").first().click();
  await binary.getByText("바이너리 파일입니다.", { exact: true }).waitFor();
  assert.equal(await binary.getByTestId("snapshot-diff").count(), 0);
  await page.getByTitle("밝은 화면 / 어두운 화면", { exact: true }).click();
  await fallback.locator('[data-context-line="6000"]').scrollIntoViewIfNeeded();
  assert.equal(await fallback.locator('[data-context-line="6000"]').getAttribute("data-selected"), "true");
  await page.screenshot({ path: join(evidence, "bounded-fallback-dark.png"), animations: "disabled" });
  console.log("PASS exact old-side context, expansion, selection across mode/wrap, stale retention, keyboard search, input during 100-file diff");
  console.log(JSON.stringify(report));
} catch (error) {
  console.error(error);
  const page = await app.firstWindow();
  await writeFile(join(evidence, "failure.txt"), await page.locator("body").innerText());
  await page.screenshot({ path: join(evidence, "failure.png"), animations: "disabled", timeout: 10_000 }).catch((failure) => console.error(failure));
  throw error;
} finally {
  const exited = once(app.process(), "exit", { signal: AbortSignal.timeout(10_000) });
  for (const window of app.windows()) await window.close();
  await exited; await gateway.close();
  await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 });
}
