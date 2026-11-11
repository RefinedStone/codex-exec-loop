// 화면 시험 (plan 22): 빌드한 앱을 가짜 Gateway 에 붙여 "관련 문서" 패널을 확인한다. 실제 codex 와 턴은 쓰지 않는다.
// 스레드를 열면 패널을 열지 않아도 그 스레드의 문서를 읽고, 버튼에 개수를 보이고, 패널은 종류별로 보여 준다.
// 읽은 뒤에 바뀐 문서는 "바뀜"을 보이고, 그 문서를 열면 표시가 사라진다.
// 창은 숨긴 채로 돌리고 패널의 글을 출력한다 (숨긴 창 텍스트 캡처).
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);

// approval-thread-list 녹화의 스레드이다.
const THREAD_ID = "01a1abfb-8f0a-7581-a8e4-c2953347f470";
const document = (kind, key, url, title, source) => ({ kind, key, url, title, source, firstSeenAt: "2026-11-11T00:00:00Z" });
const threadDocuments = {
  [THREAD_ID]: [
    document("jira", "sample.atlassian.net/ABC-123", "https://sample.atlassian.net/browse/ABC-123", "ABC-123", "user"),
    document("notion", "0123456789abcdef0123456789abcdef", "https://www.notion.so/Sample-spec-0123456789abcdef0123456789abcdef", "Sample spec", "user"),
    document("github", "sample/widgets#42", "https://github.com/sample/widgets/pull/42", "sample/widgets PR #42", "tool"),
    document("jira", "sample.atlassian.net/ABC-124", "https://sample.atlassian.net/browse/ABC-124", "ABC-124", "agent"),
  ],
};

const endpoint = fake.uniqueTestEndpoint();
// 가짜 Gateway 는 버전 확인(`akra/context/refresh`)에서 이 문서가 바뀐 것으로 답한다.
const CHANGED_KEY = "sample/widgets#42";
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  threadDocuments,
  changedThreadDocuments: { [THREAD_ID]: [CHANGED_KEY] },
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
  check("without an open thread nothing is collected", !gateway.receivedMethods.includes("akra/context/thread"));

  await page.getByTestId("thread-entry").first().click();
  const toggle = page.getByTestId("documents-toggle");
  await toggle.filter({ hasText: "문서 4" }).waitFor({ timeout: 15_000 });
  check("opening a thread collects its documents with the panel closed", gateway.receivedMethods.includes("akra/context/thread"));
  await toggle.filter({ hasText: "문서 4 · 바뀜 1" }).waitFor({ timeout: 15_000 });
  check("the version check runs after the list and the button shows the changed count", gateway.receivedMethods.indexOf("akra/context/refresh") > gateway.receivedMethods.indexOf("akra/context/thread"));

  await toggle.click();
  await page.getByTestId("documents-panel").waitFor({ timeout: 10_000 });
  check("documents are grouped by kind in first-seen order", (await page.getByTestId("documents-group").allInnerTexts()).map((text) => text.split("\n")[0]).join("|") === "Jira (2)|Notion (1)|GitHub (1)");
  check("each document is a link to its URL", (await page.getByTestId("documents-item").first().getByRole("link").getAttribute("href")) === "https://sample.atlassian.net/browse/ABC-123");
  const changedItems = page.getByTestId("documents-item").filter({ has: page.getByTestId("documents-changed") });
  check("only the changed document shows the mark", (await changedItems.allInnerTexts()).join("|").startsWith("sample/widgets PR #42"));

  console.log("--- documents panel text (one changed document) ---");
  console.log(await page.getByTestId("documents-panel").innerText());
  console.log("----------------------------");

  // 링크의 기본 동작(기본 브라우저 열기)은 막고 누른다. 읽음 처리는 그대로 일어난다.
  await page.evaluate(() => document.addEventListener("click", (event) => event.target instanceof Element && event.target.closest("a") && event.preventDefault(), true));
  await changedItems.getByRole("link").click();
  await page.getByTestId("documents-changed").waitFor({ state: "detached", timeout: 10_000 });
  check("opening the changed document clears the mark", gateway.receivedMethods.includes("akra/context/markRead"));
  await toggle.click();
  await toggle.filter({ hasText: /^문서 4$/ }).waitFor({ timeout: 10_000 });
  check("the button no longer shows a changed count", true);
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
