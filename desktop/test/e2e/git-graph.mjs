// 화면 시험 (plan 27 G1): 빌드한 앱을 가짜 Gateway 에 붙여 "커밋 그래프" 패널을 확인한다. 실제 codex, 턴, git 은 쓰지
// 않는다. 스레드를 열고 패널을 열면 그 스레드의 폴더로 그래프를 읽고, 라벨과 선을 그리고, 끝에 가까워지면 다음 쪽을
// 읽는다. 창은 숨긴 채로 돌린다. `AKRA_QA_SCREENSHOT=<png>` 이면 패널을 찍는다.
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

// 첫 쪽: 머지 커밋 하나, 두 레인, 그 뒤로 한 줄 이력이다. 번호가 클수록 새 커밋이다.
function firstPage() {
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
  return {
    repository: true,
    stale: false,
    head: sha(total),
    refs: [
      { name: "main", kind: "local_branch", target: sha(total), head: true },
      { name: "origin/main", kind: "remote_branch", target: sha(total), head: false },
      { name: "origin/topic", kind: "remote_branch", target: sha(total - 1), head: false },
      { name: "v1.0", kind: "tag", target: sha(total - 3), head: false },
    ],
    rows: rows.map((row, index) => ({ ...row, authorName: "Sample Author", authorTime: NOW - index * 3600 })),
    next: { skip: FIRST_PAGE_ROWS, lanes: [null, sha(SECOND_PAGE_ROWS)], refsDigest: "sample" },
  };
}

function secondPage() {
  const rows = [];
  for (let number = SECOND_PAGE_ROWS; number >= 1; number -= 1) {
    rows.push({ id: sha(number), parents: number > 1 ? [sha(number - 1)] : [], subject: `Commit ${number}`, lane: 1, lanesIn: [1], lanesOut: number > 1 ? [1] : [], lanesThrough: [], authorName: "Sample Author", authorTime: NOW - 90 * 86400 });
  }
  return { repository: true, stale: false, head: sha(FIRST_PAGE_ROWS + SECOND_PAGE_ROWS), refs: [], rows, next: null };
}

const graphRequests = [];
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  gitGraph(params) {
    graphRequests.push(params);
    return params.cursor ? secondPage() : firstPage();
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
