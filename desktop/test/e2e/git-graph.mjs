// 화면 시험 (plan 27 G1, G2): 빌드한 앱을 가짜 Gateway 에 붙여 "커밋 그래프" 패널을 확인한다. 실제 codex, 턴, git 은
// 모델 호출은 하지 않는다. 마지막 회귀 시험만 임시 git 저장소의 실제 변경 목록을 읽는다. 스레드를 열고 패널을 열면 그 스레드의 폴더로 그래프를 읽고, 라벨과 선을 그리고, 끝에 가까워지면 다음
// 쪽을 읽는다. 저장소가 바뀌면 누르지 않아도 다시 읽고, 줄을 고르면 상세를 보이고, 읽지 않은 쪽까지 찾는다 (G2).
// 고른 커밋에서 작업의 계획을 보고, 실행하고, 되돌린다 (G3). 가짜 Gateway 는 저장소를 고치지 않는다.
// 패널을 최대화하면 ref 목록, 그래프, 상세가 함께 보이고, 상태 띠가 끝나지 않은 작업을 한 줄로 쓴다 (#2851).
// 창은 숨긴 채로 돌린다. `AKRA_QA_SCREENSHOT=<png>` 이면 패널을 찍는다.
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);
import { graphFixture } from "./fixtures/gitGraphGateway.mjs";
import { BACKUP_ONLY_ROW, FIRST_PAGE_ROWS, SECOND_PAGE_ROWS, sha } from "./fixtures/gitGraphPages.mjs";
const { handlers, state, planRequests, executeRequests, undoRequests, journal, graphRequests, commitRequests } = graphFixture();
// IPC의 실제 git status도 읽을 수 있도록 녹화의 cwd를 격리된 빈 저장소로 바꾼다.
const fixtureRoot = mkdtempSync(join(tmpdir(), "akra-graph-ui-"));
const workspace = join(fixtureRoot, "repo");
mkdirSync(workspace);
execFileSync("git", ["init", "-q"], { cwd: workspace, windowsHide: true });
const recordings = [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING].map((source, index) => {
  const target = join(fixtureRoot, `recording-${index}.jsonl`);
  writeFileSync(target, readFileSync(source, "utf8").replaceAll("/work/example-app", workspace.replaceAll("\\", "/")));
  return target;
});
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, recordings, handlers);
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: join(fixtureRoot, "user-data") };
delete env.ELECTRON_RUN_AS_NODE;

const failures = [];
const check = (name, ok) => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}`);
  if (!ok) failures.push(name);
};

const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  check("the graph is not read while the panel is closed", graphRequests.length === 0);

  await page.getByTestId("git-graph-toggle").click();
  await page.getByTestId("git-graph-panel").waitFor({ timeout: 10_000 });
  await page.getByTestId("git-graph-row").first().waitFor({ timeout: 15_000 });
  check("the first page is requested with the folder of the open thread", graphRequests.length === 1 && typeof graphRequests[0].cwd === "string" && graphRequests[0].cwd.length > 0 && graphRequests[0].cursor === undefined);
  check("an initially stale first page is retried before showing the graph", state.staleFirstPages === 1);
  check("the header shows the checked-out branch", (await page.getByTestId("git-graph-head").innerText()) === "main");

  const firstRow = page.getByTestId("git-graph-row").first();
  check("the newest commit is the first row", (await firstRow.innerText()).includes("Merge topic into main"));
  const firstLabels = firstRow.getByTestId("git-graph-ref");
  check("origin/main is merged into the main label", (await firstLabels.count()) === 1 && (await firstLabels.first().getAttribute("title")) === "main (origin 와 같은 커밋)");
  check("a remote branch without a local branch keeps its own label", (await page.getByTestId("git-graph-row").nth(1).getByTestId("git-graph-ref").allInnerTexts()).join("|") === "origin/topic");
  check("the merge commit draws a line to each parent lane", (await firstRow.locator("svg path").count()) === 2);
  check("only visible rows are drawn", (await page.getByTestId("git-graph-row").count()) < FIRST_PAGE_ROWS);

  console.log("--- git graph panel text (top rows) ---");
  console.log((await page.getByTestId("git-graph-row").allInnerTexts()).slice(0, 6).map((text) => text.replace(/\n+/g, " | ")).join("\n"));
  console.log("----------------------------");
  if (process.env.AKRA_QA_SCREENSHOT) await page.screenshot({ path: process.env.AKRA_QA_SCREENSHOT });

  await page.getByTestId("git-graph-scroll").evaluate((element) => element.scrollTo(0, element.scrollHeight));
  await page.getByTestId("git-graph-count").filter({ hasText: `${FIRST_PAGE_ROWS + SECOND_PAGE_ROWS}개` }).waitFor({ timeout: 15_000 });
  check("scrolling near the end reads the next page with the cursor of the first page", graphRequests.length === 2 && graphRequests[1].cursor?.skip === FIRST_PAGE_ROWS && graphRequests[1].cursor?.refsDigest === "sample");
  await page.getByTestId("git-graph-scroll").evaluate((element) => element.scrollTo(0, element.scrollHeight));
  await page.getByTestId("git-graph-row").filter({ hasText: "Commit 1" }).last().waitFor({ timeout: 10_000 });
  check("the last page ends the reading", graphRequests.length === 2);

  await page.getByTestId("git-graph-refresh").click();
  await page.getByTestId("git-graph-count").filter({ hasText: `${FIRST_PAGE_ROWS}개 읽음` }).waitFor({ timeout: 15_000 });
  check("refresh reads the first page again", graphRequests.length === 3 && graphRequests[2].cursor === undefined);

  // ---- G2 ----
  const rowWith = (text) => page.getByTestId("git-graph-row").filter({ hasText: text });
  const total = FIRST_PAGE_ROWS + SECOND_PAGE_ROWS;
  check("a backup ref has its own label shape", (await page.locator(".git-ref-backup").first().innerText()) === "main/op-1");
  check("a commit reached only from a backup is dim", (await page.getByTestId("git-graph-row").nth(BACKUP_ONLY_ROW).getAttribute("class")).includes("git-row-dim"));

  const badge = rowWith("Topic work").getByTestId("git-graph-thread");
  check("the commit of the thread's worktree carries a thread badge", (await badge.count()) === 1 && (await badge.innerText()).trim().length > 0);
  await badge.click();
  check("the badge opens the thread and does not select the row", (await page.getByTestId("git-commit-detail").count()) === 0 && commitRequests.length === 0);

  // 저장소가 바뀐다. 누르지 않아도 새 커밋이 보이고, 보던 줄은 같은 자리에 남는다.
  await page.getByTestId("git-graph-scroll").evaluate((element) => element.scrollTo(0, 10 * 28));
  const topBefore = await page.getByTestId("git-graph-scroll").evaluate((element) => element.scrollTop);
  const polledBefore = state.refsRequests;
  // 연속 stale로 첫 polling 시도가 끝나도 기존 DOM과 읽은 줄 수/스크롤을 유지한다.
  const keptScroll = await page.getByTestId("git-graph-scroll").elementHandle();
  const staleBefore = state.staleFirstPages;
  const repeatedStale = new Promise((resolve) => {
    state.onStaleFirstPage = () => { if (state.staleFirstPages >= staleBefore + 4) resolve(); };
  });
  state.holdFirstPagesStale = true;
  state.repositoryChanged = true;
  let staleTimer;
  await Promise.race([
    repeatedStale,
    new Promise((_, reject) => { staleTimer = setTimeout(() => reject(new Error("stale polling did not repeat")), 20_000); }),
  ]).finally(() => clearTimeout(staleTimer));
  check("a stale first polling page preserves the existing graph rows and scroll node",
    (await page.getByTestId("git-graph-count").innerText()) === `${FIRST_PAGE_ROWS}개 읽음`
      && await keptScroll.evaluate((element, top) => element.isConnected && element.scrollTop === top, topBefore));
  state.holdFirstPagesStale = false;
  await page.getByTestId("git-graph-count").filter({ hasText: `${FIRST_PAGE_ROWS + 1}개 읽음` }).waitFor({ timeout: 20_000 });
  check("the panel watches the refs and reads again without a click", state.refsRequests > polledBefore && graphRequests.length === 4 && graphRequests[3].cursor === undefined && graphRequests[3].limit === FIRST_PAGE_ROWS);
  const topAfter = await page.getByTestId("git-graph-scroll").evaluate((element) => element.scrollTop);
  check("the rows on the screen stay in place after the new commit arrives", topBefore === 10 * 28 && topAfter === 11 * 28);
  await page.getByTestId("git-graph-scroll").evaluate((element) => element.scrollTo(0, 0));
  check("the new commit is the first row", (await page.getByTestId("git-graph-row").first().innerText()).includes("New commit"));

  // 줄을 고르면 상세가 나온다.
  // 줄의 가운데를 누르지 않는다. 좁은 패널에서는 가운데에 스레드 배지가 온다. 배지는 줄을 고르지 않는다.
  await rowWith("Topic work").locator(".git-row-time").click();
  await page.getByTestId("git-commit-message").waitFor({ timeout: 10_000 });
  check("selecting a row reads that commit", commitRequests.length === 1 && commitRequests[0] === sha(total - 1));
  check("the detail shows the whole message", (await page.getByTestId("git-commit-message").innerText()).includes("The body of the topic commit."));
  const detail = page.getByTestId("git-commit-detail");
  check("the detail lists the changed file with its counts", (await detail.getByTestId("diff-file").count()) === 1 && (await detail.getByTestId("diff-file").innerText()).includes("src/topic.ts"));
  check("the selected row is marked", (await rowWith("Topic work").getAttribute("aria-selected")) === "true");
  // 숨긴 창에서는 `page.screenshot` 이 끝나지 않을 수 있다. 창의 내용을 직접 찍는다.
  // 숨긴 창은 그리기를 미룬다. 한 번 찍어 그리게 하고, 그 다음 장면을 쓴다.
  const capture = () => app.evaluate(async ({ BrowserWindow }) => (await BrowserWindow.getAllWindows()[0].webContents.capturePage()).toPNG().toString("base64"));
  if (process.env.AKRA_QA_SCREENSHOT) {
    await capture();
    await page.waitForTimeout(300);
    const png = await capture();
    writeFileSync(process.env.AKRA_QA_SCREENSHOT.replace(/\.png$/, "-g2.png"), Buffer.from(png, "base64"));
  }
  await detail.getByTestId("git-commit-parent").click();
  await page.getByTestId("git-commit-message").filter({ hasText: "Fork point" }).waitFor({ timeout: 10_000 });
  check("a parent in the detail selects that commit", commitRequests[1] === sha(total - 3) && (await rowWith("Fork point").getAttribute("aria-selected")) === "true");
  await page.getByTestId("git-graph-scroll").focus();
  await page.keyboard.press("ArrowUp");
  await page.getByTestId("git-commit-message").filter({ hasText: "Another commit" }).waitFor({ timeout: 10_000 });
  check("the arrow keys move the selection", (await rowWith("Main work").getAttribute("aria-selected")) === "true");
  await page.getByTestId("git-commit-close").click();
  check("closing the detail clears the selection", (await page.getByTestId("git-commit-detail").count()) === 0 && (await page.locator(".git-row-selected").count()) === 0);

  // 찾기: 읽은 줄에서 찾고, 없으면 다음 쪽을 읽어 찾는다.
  const find = page.getByTestId("git-graph-find");
  await find.fill("topic work");
  check("find counts the matches in the loaded rows", (await page.getByTestId("git-graph-find-count").innerText()).startsWith("1개"));
  check("rows that do not match are dim", (await rowWith("Main work").getAttribute("class")).includes("git-row-dim") && !(await rowWith("Topic work").getAttribute("class")).includes("git-row-dim"));
  await find.press("Enter");
  check("Enter selects the match", (await rowWith("Topic work").getAttribute("aria-selected")) === "true");
  const requestsBeforeFind = graphRequests.length;
  await find.fill("Commit 3");
  check("a text only on the next page is not found in the loaded rows", (await page.getByTestId("git-graph-find-count").innerText()).startsWith("없음"));
  await page.getByTestId("git-graph-find-next").click();
  await page.locator(".git-row-selected").filter({ hasText: "Commit 39" }).waitFor({ timeout: 15_000 });
  const findRequest = graphRequests[requestsBeforeFind];
  check("next reads further pages until it finds the text", graphRequests.length === requestsBeforeFind + 1 && findRequest.cursor?.skip === FIRST_PAGE_ROWS && findRequest.limit === 1000);

  // ---- G3 ----
  await find.fill("");
  await page.getByTestId("git-graph-scroll").evaluate((element) => element.scrollTo(0, 0));
  const chooser = page.getByRole("dialog", { name: "선택한 커밋 작업" });
  await rowWith("Main work").locator(".git-row-time").click();
  await rowWith("Fork point").locator(".git-row-time").click({ button: "right" });
  await chooser.waitFor();
  check("right-click targets the clicked commit, not the previous selection", (await rowWith("Fork point").getAttribute("aria-selected")) === "true" && await chooser.getByTestId("git-action-reset").isVisible());
  await page.keyboard.press("Escape");
  await chooser.waitFor({ state: "detached" });
  const mod = process.platform === "darwin" ? "Meta" : "Control";
  await page.keyboard.press(mod + "+g");
  await chooser.waitFor();
  await chooser.getByRole("button", { name: "닫기" }).click();
  await page.keyboard.press(mod + "+k");
  await page.getByTestId("palette").getByRole("button", { name: /선택한 커밋 작업/ }).click();
  await chooser.waitFor();

  const reset = page.getByTestId("git-action-reset");
  await reset.waitFor({ timeout: 10_000 });
  check("the selected commit offers a reset of the checked-out branch", (await reset.innerText()) === "여기로 reset (main)");
  check("no operation log is shown before the first operation", (await page.getByTestId("git-operation-log").count()) === 0);
  await chooser.getByTestId("git-action-reset").click();
  await page.getByTestId("git-plan").waitFor({ timeout: 10_000 });
  check("a click asks for a plan and changes nothing", planRequests.length === 1 && planRequests[0].operation.kind === "reset" && planRequests[0].operation.target === sha(total - 3) && executeRequests.length === 0);
  const planned = page.getByTestId("git-graph-planned");
  check("the open plan draws the branch at its new commit with a dashed label", (await planned.count()) === 1 && (await planned.innerText()) === "main" && (await rowWith("Fork point").getByTestId("git-graph-planned").count()) === 1 && (await rowWith("Fork point").locator(".git-node-planned").count()) === 1);
  const planLines = (await page.getByTestId("git-plan-line").allInnerTexts()).join("\n");
  check("the plan says how many commits leave the branch and what the backup keeps", planLines.includes("브랜치에서 커밋 4개가 빠집니다.") && planLines.includes("그중 2개는 다른 브랜치와 태그가 닿지 못합니다. 백업 ref 가 지킵니다."));

  await page.getByTestId("git-plan-run").click();
  await page.getByTestId("git-plan").filter({ hasText: "저장소가 바뀌어서 실행하지 않았습니다." }).waitFor({ timeout: 10_000 });
  check("the execution carries the expectation of the plan", executeRequests.length === 1 && executeRequests[0].expectation === "seen-at-plan");
  check("a stale plan is not run; the new plan is shown", (await page.getByTestId("git-operation-notice").count()) === 0 && journal.length === 0);

  await page.getByTestId("git-plan-run").click();
  await page.getByTestId("git-operation-notice").waitFor({ timeout: 10_000 });
  check("the second execution carries the expectation of the new plan", executeRequests.length === 2 && executeRequests[1].expectation === "seen-after-change");
  check("the notice names the operation and offers undo", (await page.getByTestId("git-operation-notice").innerText()).includes("reset: main → ") && (await page.getByTestId("git-operation-notice").innerText()).includes("· 완료") && (await page.getByTestId("git-operation-undo").count()) === 1);
  check("the plan closes after the execution", (await page.getByTestId("git-plan").count()) === 0);
  check("the dashed preview leaves with the plan", (await page.getByTestId("git-graph-planned").count()) === 0);
  await page.getByTestId("git-operation-log").waitFor({ timeout: 10_000 });
  await page.getByTestId("git-operation-log").locator("summary").click();
  check("the operation log lists the operation", (await page.getByTestId("git-operation-entry").count()) === 1);
  if (process.env.AKRA_QA_SCREENSHOT) {
    await capture();
    await page.waitForTimeout(300);
    writeFileSync(process.env.AKRA_QA_SCREENSHOT.replace(/\.png$/, "-g3.png"), Buffer.from(await capture(), "base64"));
  }

  await page.getByTestId("git-operation-undo").click();
  await page.getByTestId("git-operation-notice").filter({ hasText: "되돌리기 (reset" }).waitFor({ timeout: 10_000 });
  check("undo names the operation of the notice", undoRequests.length === 1 && undoRequests[0] === "20261119T010203Z-0000002a");
  check("an undo and an undone operation offer no further undo", (await page.getByTestId("git-operation-undo").count()) === 0 && (await page.getByTestId("git-operation-entry").count()) === 2 && (await page.getByTestId("git-operation-entry").locator("button").count()) === 0);

  await rowWith("Main work").getByRole("checkbox").check();
  await rowWith("Fork point").getByRole("checkbox").check();
  await page.getByRole("button", { name: "cherry-pick 계획 (2)" }).click();
  await page.getByTestId("git-plan-line").filter({ hasText: "충돌 예상: a.txt" }).waitFor();
  check("multi-select previews the submitted commits without execution", planRequests.at(-1).operation.commits.join(",") === [sha(total - 2),sha(total - 3)].join(",") && executeRequests.length === 2);
  await page.getByTestId("git-plan-cancel").click();
  // ---- #2851: 상태 띠, 최대화, ref 목록 ----
  check("the narrow panel has no ref list", (await page.getByTestId("git-ref-list").count()) === 0 && (await page.getByTestId("right-panel").getAttribute("data-maximized")) === "false");
  check("no status strip while nothing is unfinished", (await page.getByTestId("git-status-strip").count()) === 0);
  state.worktreeProgress = { operationInProgress: "rebase", conflictedFiles: 2 };
  const strip = page.getByTestId("git-status-strip");
  await strip.waitFor({ timeout: 10_000 });
  check("the status strip writes the unfinished operation and its conflicts in one line",
    (await strip.innerText()) === "rebase 진행 중 · 충돌 파일 2개" && (await strip.evaluate((element) => element.getBoundingClientRect().height <= 28)));

  await page.getByTestId("graph-maximize").click();
  const refList = page.getByTestId("git-ref-list");
  await refList.waitFor({ timeout: 10_000 });
  const refListText = await refList.innerText();
  check("the ref list groups the local, remote, tag, backup and worktree refs", ["로컬", "원격", "태그", "백업", "worktree", "origin/topic", "v1.0", "main/op-1"].every((text) => refListText.includes(text)));
  await refList.getByTestId("git-ref-item").filter({ hasText: "origin/topic" }).click();
  await page.getByTestId("git-commit-message").filter({ hasText: "Topic work" }).waitFor({ timeout: 10_000 });
  const box = async (testId) => page.getByTestId(testId).boundingBox();
  const [listBox, graphBox, detailBox, stripBox] = [await box("git-ref-list"), await box("git-graph-scroll"), await box("git-commit-detail"), await box("git-status-strip")];
  check("maximized: the ref list is left of the graph and the detail is below it, all with the status strip",
    (await page.getByTestId("right-panel").getAttribute("data-maximized")) === "true"
      && listBox && graphBox && detailBox && stripBox
      && listBox.x + listBox.width <= graphBox.x && detailBox.y >= graphBox.y + graphBox.height - 1 && stripBox.y < graphBox.y
      && (await rowWith("Topic work").getAttribute("aria-selected")) === "true");
  if (process.env.AKRA_QA_SCREENSHOT) {
    await capture();
    await page.waitForTimeout(300);
    writeFileSync(process.env.AKRA_QA_SCREENSHOT.replace(/\.png$/, "-maximized.png"), Buffer.from(await capture(), "base64"));
  }
  await page.getByTestId("graph-restore").click();
  check("restoring the panel removes the ref list and keeps the graph", (await page.getByTestId("git-ref-list").count()) === 0 && (await page.getByTestId("git-graph-row").count()) > 0);
  state.worktreeProgress = { operationInProgress: null, conflictedFiles: 0 };
  await strip.waitFor({ state: "detached", timeout: 10_000 });
  check("the status strip leaves when the work is finished", (await strip.count()) === 0);
  await page.getByText("그래프 필터", { exact:true }).click();
  await page.getByLabel("작성자",{exact:true}).fill("needle");
  await page.getByLabel("일치하지 않는 커밋 숨기기").check();
  await page.getByRole("button",{name:"필터 적용"}).click();
  await page.getByTestId("git-graph-count").filter({hasText:/^2개$/}).waitFor();
  check("hidden rows request later server pages with the same filter", graphRequests.at(-1).cursor && graphRequests.at(-1).filter.author === "needle" && graphRequests.at(-1).filter.hide === true);
  await page.getByRole("button",{name:"초기화",exact:true}).click();
  // #2851 review: a real unborn repository with two changed files still has a clickable graph row.
  state.unborn = true;
  writeFileSync(join(workspace, "first.txt"), "first\n");
  writeFileSync(join(workspace, "second.txt"), "second\n");
  await page.getByTestId("review-toggle").click();
  const workingTree = page.getByTestId("working-tree");
  await workingTree.locator(".review-toolbar").getByRole("button", { name: "새로 고침", exact: true }).click();
  await workingTree.getByTestId("working-tree-summary").filter({ hasText: "파일 2개" }).waitFor({ timeout: 15_000 });
  await page.getByTestId("git-graph-toggle").click();
  await page.getByTestId("git-graph-count").filter({ hasText: "0개" }).waitFor({ timeout: 15_000 });
  const uncommitted = page.getByTestId("git-graph-uncommitted");
  await uncommitted.waitFor({ timeout: 10_000 });
  check("an unborn repository shows the uncommitted row and no commit rows",
    (await uncommitted.innerText()).includes("파일 2개")
      && (await page.getByTestId("git-graph-row").count()) === 0
      && await page.getByText("이 저장소에는 아직 커밋이 없습니다.", { exact: true }).isVisible());
  await uncommitted.click();
  await workingTree.getByTestId("working-tree-summary").filter({ hasText: "파일 2개" }).waitFor({ timeout: 10_000 });
  check("the unborn repository row opens the changes panel", (await workingTree.getByTestId("git-file").count()) === 2);
  check("the panel never starts a turn", !gateway.receivedMethods.includes("turn/start"));
} catch (error) {
  console.error(error);
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  // 창을 닫으면 앱이 끝난다. app.close() 는 쓰지 않는다 (P2 기록 참고).
  const exited = new Promise((resolve) => app.process().once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  await gateway.close();
  rmSync(fixtureRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 });
}
process.exit(failures.length === 0 ? 0 : 1);
