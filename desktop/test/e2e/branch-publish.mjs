// 게시 화면을 실제 Electron 에서 확인한다. 원격 쓰기는 가짜 IPC 응답으로 막는다.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { once } from "node:events";
import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "../..");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  kernel: { kernel: "fixture-repo", queries: {}, onRequest(method, params) {
    if (method !== "akra/kernel/query") return;
    if (params.query?.type === "feature_bindings") {
      return { result: { type: "feature_bindings", repository: "sample/widgets", pr_review: "observe" } };
    }
    if (params.query?.type !== "pr_review_result") return;
    assert.equal(params.query.number, 12);
    return { result: { type: "pr_review_result", number: 12, review: {
      head: "a".repeat(40),
      result: { status: "reviewed", backend: "fixture-review", result: {
        summary: "One issue in the fixture.",
        findings: [{ path: "src/example.rs", line: 23, severity: "medium", body: "<b>Keep this finding as text.</b>" }],
      } },
    } } };
  } },
});
const userData = await mkdtemp(join(tmpdir(), "akra-publish-ui-"));
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;
const app = await electron.launch({ executablePath: createRequire(import.meta.url)("electron"), args: [desktopDir], env });
try {
  await app.evaluate(({ ipcMain }) => {
    let pushed = false;
    let created = false;
    for (const channel of ["akra:git-status", "akra:git-branch-summary", "akra:git-push", "akra:git-create-pull-request"]) ipcMain.removeHandler(channel);
    ipcMain.handle("akra:git-status", () => ({ ok: true, branch: "feat/example", files: [], truncated: false }));
    ipcMain.handle("akra:git-branch-summary", () => ({
      ok: true, branch: "feat/example", remoteBranch: pushed ? "origin/feat/example" : null,
      repository: "sample/widgets", base: "origin/main", truncated: false, pullRequestError: null,
      commits: [{ sha: "abc1234", shortSha: "abc1234", subject: "Add example", author: "Tester", committedAt: 0, pushed }],
      pullRequest: created ? { number: 12, title: "Add example", url: "https://github.com/sample/widgets/pull/12", state: "OPEN", isDraft: true, baseBranch: "main", checks: null } : null,
    }));
    ipcMain.handle("akra:git-push", () => {
      pushed = true;
      return { ok: true, url: null };
    });
    ipcMain.handle("akra:git-create-pull-request", (_event, _cwd, draft) => {
      if (draft.title !== "Add example" || draft.base !== "main" || draft.draft !== true) throw new Error("Unexpected PR draft");
      created = true;
      return { ok: true, url: null };
    });
  });
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("review-toggle").click();
  await page.getByTestId("branch-push").waitFor();
  assert.equal(await page.getByTestId("branch-pr-new").count(), 0);
  await page.getByTestId("branch-push").click();
  await page.getByTestId("branch-pr-new").waitFor();
  assert.equal(await page.getByTestId("branch-push").count(), 0);
  await page.getByTestId("branch-pr-new").click();
  const form = page.getByTestId("branch-pr-form");
  await form.waitFor();
  assert.equal(await form.getByLabel("제목", { exact: true }).inputValue(), "Add example");
  assert.equal(await form.getByLabel("대상 브랜치").inputValue(), "main");
  await form.getByLabel("초안으로 만들기").check();
  if (process.env.AKRA_QA_SCREENSHOT) {
    await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].showInactive());
    await page.screenshot({ path: process.env.AKRA_QA_SCREENSHOT, animations: "disabled" });
  }
  await form.getByRole("button", { name: "PR 만들기", exact: true }).click();
  await page.getByTestId("branch-pr").waitFor();
  assert.equal(await page.getByTestId("branch-pr-new").count(), 0);
  assert.equal(await page.getByTestId("branch-pr-form").count(), 0);
  const review = page.getByTestId("pr-review-result");
  await review.getByText("One issue in the fixture.").waitFor();
  await review.getByText("src/example.rs:23").waitFor();
  await review.getByText("medium", { exact: true }).waitFor();
  await review.getByText("저장소 설정 · observe").waitFor();
  await review.getByText("<b>Keep this finding as text.</b>").waitFor();
  assert.equal(await review.locator("b").count(), 0);
  assert.ok(!gateway.receivedMethods.includes("akra/kernel/command"));
  console.log("PASS stored PR review shows summary, severity, and plain-text findings");
  console.log("PASS push refreshes the panel, PR form sends the draft, and the created PR replaces the form");
} finally {
  const exited = once(app.process(), "exit");
  for (const window of app.windows()) await window.close();
  await exited;
  await gateway.close();
  await rm(userData, { recursive: true, force: true });
}
