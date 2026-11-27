// 화면 시험 (plan 27 G1, G2): 빌드한 앱을 가짜 Gateway 에 붙여 "커밋 그래프" 패널을 확인한다. 실제 codex, 턴, git 은
// 쓰지 않는다. 스레드를 열고 패널을 열면 그 스레드의 폴더로 그래프를 읽고, 라벨과 선을 그리고, 끝에 가까워지면 다음
// 쪽을 읽는다. 저장소가 바뀌면 누르지 않아도 다시 읽고, 줄을 고르면 상세를 보이고, 읽지 않은 쪽까지 찾는다 (G2).
// 고른 커밋에서 작업의 계획을 보고, 실행하고, 되돌린다 (G3). 가짜 Gateway 는 저장소를 고치지 않는다.
// 패널을 최대화하면 ref 목록, 그래프, 상세가 함께 보이고, 상태 띠가 끝나지 않은 작업을 한 줄로 쓴다 (#2851).
// 창은 숨긴 채로 돌린다. `AKRA_QA_SCREENSHOT=<png>` 이면 패널을 찍는다.
import { writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);

// 첫 쪽은 화면보다 훨씬 길다. 패널은 읽은 줄의 끝에서 60 줄 앞에 오면 다음 쪽을 읽는다.
const FIRST_PAGE_ROWS = 200;
const SECOND_PAGE_ROWS = 40;
const sha = (number) => number.toString(16).padStart(40, "0");
const NOW = Math.floor(Date.now() / 1000);

// 저장소가 바뀐 뒤이다. 맨 위에 새 커밋이 하나 있고 ref 들의 지문이 다르다.
let repositoryChanged = false;
// `akra/git/refs` 가 주는 작업 트리의 끝나지 않은 일이다 (#2851).
let worktreeProgress = { operationInProgress: null, conflictedFiles: 0 };
const BACKUP_ONLY_ROW = 6;

// 첫 쪽: 머지 커밋 하나, 두 레인, 그 뒤로 한 줄 이력이다. 번호가 클수록 새 커밋이다.
function firstPage(cwd) {
  const total = FIRST_PAGE_ROWS + SECOND_PAGE_ROWS;
  const rows = [
    { id: sha(total), parents: [sha(total - 2), sha(total - 1)], subject: "Merge topic into main", lane: 0, lanesIn: [], lanesOut: [0, 1], lanesThrough: [] },
    { id: sha(total - 1), parents: [sha(total - 3)], subject: "Topic work", lane: 1, lanesIn: [1], lanesOut: [1], lanesThrough: [0] },
    { id: sha(total - 2), parents: [sha(total - 3)], subject: "Main work", lane: 0, lanesIn: [0], lanesOut: [1], lanesThrough: [1] },
    { id: sha(total - 3), parents: [sha(total - 4)], subject: "Fork point", lane: 1, lanesIn: [1], lanesOut: [1], lanesThrough: [] },
  ];
  for (let number = total - 4; rows.length < FIRST_PAGE_ROWS; number -= 1) {
    rows.push({ id: sha(number), parents: [sha(number - 1)], subject: `Commit ${number}`, lane: 1, lanesIn: [1], lanesOut: [1], lanesThrough: [] });
  }
  rows[BACKUP_ONLY_ROW].backupOnly = true;
  if (repositoryChanged) rows.unshift({ id: sha(total + 1), parents: [sha(total)], subject: "New commit", lane: 0, lanesIn: [], lanesOut: [0], lanesThrough: [] });
  const head = rows[0].id;
  return {
    repository: true,
    stale: false,
    head,
    refsDigest: repositoryChanged ? "d2" : "d1",
    refs: [
      { name: "main", kind: "local_branch", target: head, head: true },
      { name: "origin/main", kind: "remote_branch", target: sha(total), head: false },
      { name: "origin/topic", kind: "remote_branch", target: sha(total - 1), head: false },
      { name: "v1.0", kind: "tag", target: sha(total - 3), head: false },
      { name: "main/op-1", kind: "backup", target: rows[repositoryChanged ? BACKUP_ONLY_ROW + 1 : BACKUP_ONLY_ROW].id, head: false },
    ],
    // 열린 스레드의 폴더가 작업 트리이다. 그 작업 트리는 "Topic work" 를 체크아웃했다.
    worktrees: [{ path: String(cwd).replaceAll("\\", "/"), head: sha(total - 1), branch: "topic" }],
    rows: rows.map((row, index) => ({ ...row, authorName: "Sample Author", authorTime: NOW - index * 3600 })),
    next: { skip: FIRST_PAGE_ROWS, lanes: [null, sha(SECOND_PAGE_ROWS)], refsDigest: "sample" },
  };
}

const COMMIT_DIFF = ["diff --git a/src/topic.ts b/src/topic.ts", "index 1111111..2222222 100644", "--- a/src/topic.ts", "+++ b/src/topic.ts", "@@ -1,2 +1,3 @@", " export const topic = 1;", "+export const more = 2;", " // end", ""].join("\n");

function commitDetail(commit) {
  const total = FIRST_PAGE_ROWS + SECOND_PAGE_ROWS;
  const topic = commit === sha(total - 1);
  return {
    commit: {
      id: commit,
      parents: topic ? [sha(total - 3)] : [],
      authorName: "Sample Author",
      authorEmail: "author@example.com",
      authorTime: NOW - 3600,
      committerName: "Sample Committer",
      committerTime: NOW - 1800,
      message: topic ? "Topic work\n\nThe body of the topic commit." : commit === sha(total - 3) ? "Fork point" : "Another commit",
      diff: topic ? COMMIT_DIFF : "",
      diffTooLarge: false,
    },
  };
}

function secondPage() {
  const rows = [];
  for (let number = SECOND_PAGE_ROWS; number >= 1; number -= 1) {
    rows.push({ id: sha(number), parents: number > 1 ? [sha(number - 1)] : [], subject: `Commit ${number}`, lane: 1, lanesIn: [1], lanesOut: number > 1 ? [1] : [], lanesThrough: [], authorName: "Sample Author", authorTime: NOW - 90 * 86400 });
  }
  return { repository: true, stale: false, head: sha(FIRST_PAGE_ROWS + SECOND_PAGE_ROWS), refs: [], rows, next: null };
}

// ---- G3: 작업 엔진의 가짜 답 ----
const planRequests = [];
const executeRequests = [];
const undoRequests = [];
const journal = [];
// 첫 실행은 "저장소가 계획과 다르다"로 답한다. 화면은 새 계획을 보여야 한다.
let staleAnswers = 1;

function operationPlan(operation, expectation) {
  const total = FIRST_PAGE_ROWS + SECOND_PAGE_ROWS;
  return {
    operation,
    expectation,
    branch: "main",
    before: { commit: sha(total + 1), branch: "main" },
    after: { commit: operation.target, branch: "main" },
    impact: { dropped: 4, unreachable: 2, gained: 0 },
    blockers: [],
  };
}

function operationRecord(opId, operation, undoOf, cwd) {
  const total = FIRST_PAGE_ROWS + SECOND_PAGE_ROWS;
  return {
    opId,
    operation,
    // 작업을 실행한 폴더이다. 화면은 같은 폴더의 기록에만 되돌리기를 보인다.
    worktree: cwd,
    before: { commit: sha(total + 1), branch: "main" },
    after: { commit: operation.target, branch: "main" },
    backupRef: `refs/akra/backup/main/${opId}`,
    status: "done",
    startedAt: NOW,
    finishedAt: NOW,
    error: null,
    undoOf,
    undoneBy: null,
  };
}

// 첫 조회와 background 갱신에 stale 응답을 주입한다. 정상 페이지 요청 수는 별도로 센다.
let initialStalePages = 1;
let holdFirstPagesStale = false;
let staleFirstPages = 0;
let onStaleFirstPage = () => {};

const graphRequests = [];
const commitRequests = [];
let refsRequests = 0;
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  gitGraph(params) {
    if (!params.cursor && (initialStalePages > 0 || holdFirstPagesStale)) {
      initialStalePages = Math.max(0, initialStalePages - 1);
      staleFirstPages += 1;
      onStaleFirstPage();
      return { repository: true, stale: true, head: sha(FIRST_PAGE_ROWS + SECOND_PAGE_ROWS + 1), refs: [], rows: [], next: null };
    }
    graphRequests.push(params);
    return params.cursor ? secondPage() : firstPage(params.cwd);
  },
  gitRefs() {
    refsRequests += 1;
    return { repository: true, refsDigest: repositoryChanged ? "d2" : "d1", ...worktreeProgress };
  },
  gitCommit(params) {
    commitRequests.push(params.commit);
    return commitDetail(params.commit);
  },
  gitPlan(params) {
    planRequests.push(params);
    return operationPlan(params.operation, "seen-at-plan");
  },
  gitExecute(params) {
    executeRequests.push(params);
    if (staleAnswers > 0) {
      staleAnswers -= 1;
      return { status: "stale", plan: operationPlan(params.operation, "seen-after-change") };
    }
    const record = operationRecord("20261119T010203Z-0000002a", params.operation, null, params.cwd);
    journal.unshift(record);
    return { status: "done", record };
  },
  gitOperations() {
    return { operations: journal };
  },
  gitUndo(params) {
    undoRequests.push(params.opId);
    const undone = journal.find((record) => record.opId === params.opId);
    const record = operationRecord("20261119T010300Z-0000002b", { kind: "reset", target: undone.before.commit }, undone.opId, params.cwd);
    undone.undoneBy = record.opId;
    journal.unshift(record);
    return { status: "done", record };
  },
});
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1" };
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
  check("an initially stale first page is retried before showing the graph", staleFirstPages === 1);
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
  const polledBefore = refsRequests;
  // 연속 stale로 첫 polling 시도가 끝나도 기존 DOM과 읽은 줄 수/스크롤을 유지한다.
  const keptScroll = await page.getByTestId("git-graph-scroll").elementHandle();
  const staleBefore = staleFirstPages;
  const repeatedStale = new Promise((resolve) => {
    onStaleFirstPage = () => { if (staleFirstPages >= staleBefore + 4) resolve(); };
  });
  holdFirstPagesStale = true;
  repositoryChanged = true;
  let staleTimer;
  await Promise.race([
    repeatedStale,
    new Promise((_, reject) => { staleTimer = setTimeout(() => reject(new Error("stale polling did not repeat")), 20_000); }),
  ]).finally(() => clearTimeout(staleTimer));
  check("a stale first polling page preserves the existing graph rows and scroll node",
    (await page.getByTestId("git-graph-count").innerText()) === `${FIRST_PAGE_ROWS}개 읽음`
      && await keptScroll.evaluate((element, top) => element.isConnected && element.scrollTop === top, topBefore));
  holdFirstPagesStale = false;
  await page.getByTestId("git-graph-count").filter({ hasText: `${FIRST_PAGE_ROWS + 1}개 읽음` }).waitFor({ timeout: 20_000 });
  check("the panel watches the refs and reads again without a click", refsRequests > polledBefore && graphRequests.length === 4 && graphRequests[3].cursor === undefined && graphRequests[3].limit === FIRST_PAGE_ROWS);
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
  await rowWith("Fork point").locator(".git-row-time").click();
  const reset = page.getByTestId("git-action-reset");
  await reset.waitFor({ timeout: 10_000 });
  check("the selected commit offers a reset of the checked-out branch", (await reset.innerText()) === "여기로 reset (main)");
  check("no operation log is shown before the first operation", (await page.getByTestId("git-operation-log").count()) === 0);
  await reset.click();
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

  // ---- #2851: 상태 띠, 최대화, ref 목록 ----
  check("the narrow panel has no ref list", (await page.getByTestId("git-ref-list").count()) === 0 && (await page.getByTestId("right-panel").getAttribute("data-maximized")) === "false");
  check("no status strip while nothing is unfinished", (await page.getByTestId("git-status-strip").count()) === 0);
  worktreeProgress = { operationInProgress: "rebase", conflictedFiles: 2 };
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
  worktreeProgress = { operationInProgress: null, conflictedFiles: 0 };
  await strip.waitFor({ state: "detached", timeout: 10_000 });
  check("the status strip leaves when the work is finished", (await strip.count()) === 0);
  check("the panel never starts a turn", !gateway.receivedMethods.includes("turn/start"));
} catch (error) {
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  // 창을 닫으면 앱이 끝난다. app.close() 는 쓰지 않는다 (P2 기록 참고).
  const exited = new Promise((resolve) => app.process().once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  await gateway.close();
}
process.exit(failures.length === 0 ? 0 : 1);
