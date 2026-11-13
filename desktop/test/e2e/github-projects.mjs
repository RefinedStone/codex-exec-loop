// 화면 시험 (R11): 빌드한 앱을 가짜 Gateway 에 붙여 GitHub Project 패널을 확인한다. gh 와 git 은 쓰지 않는다. main 의
// GitHub IPC 응답을 가짜로 바꾼다. Project 를 저장하고, 현재 저장소로 거르고, 이슈를 넘기면 그 저장소의 폴더가
// 고른 상태로 새 스레드의 입력창이 채워진다. 다른 저장소의 이슈는 폴더를 고르지 않는다. 턴은 만들지 않는다.
import { createRequire } from "node:module";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);

const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING]);
const userData = await mkdtemp(join(tmpdir(), "akra-projects-ui-"));
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;

const failures = [];
const check = (name, ok) => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}`);
  if (!ok) failures.push(name);
};

const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  await app.evaluate(({ ipcMain }) => {
    for (const channel of ["akra:github-project", "akra:github-issue-prompt", "akra:github-repository"]) ipcMain.removeHandler(channel);
    const issue = (number, repository, status) => ({ kind: "issue", title: `Issue ${number}`, url: `https://github.com/${repository}/issues/${number}`, repository, number, status });
    ipcMain.handle("akra:github-project", (_event, project) => {
      if (project !== "sample/1") throw new Error(`Unexpected project ${project}`);
      return {
        title: "Sample board",
        url: "https://github.com/users/sample/projects/1",
        items: [
          issue(1, "sample/widgets", "Todo"),
          { kind: "pullRequest", title: "PR 2", url: "https://github.com/sample/widgets/pull/2", repository: "sample/widgets", number: 2, status: "In progress" },
          issue(3, "sample/lib", "Todo"),
          { kind: "draft", title: "Draft idea", url: null, repository: null, number: null, status: null },
        ],
        closedCount: 5,
        truncated: false,
      };
    });
    ipcMain.handle("akra:github-issue-prompt", (_event, url) => `Issue prompt for ${url}`);
    // 원격 주소의 대소문자가 GitHub 의 이름과 다른 경우이다.
    ipcMain.handle("akra:github-repository", (_event, cwd) => (cwd === "/work/example-app" ? "Sample/Widgets" : null));
  });
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();

  await page.getByTestId("work-toggle").click();
  await page.getByTestId("projects-toggle").click();
  const panel = page.getByTestId("github-project-panel");
  await panel.getByTestId("github-project-form").waitFor({ timeout: 10_000 });
  await panel.getByLabel("Project").fill("sample/1");
  await panel.getByRole("button", { name: "저장" }).click();
  await panel.getByTestId("github-project-item").first().waitFor({ timeout: 10_000 });

  const filter = panel.getByTestId("github-project-filter");
  check("the filter starts at the open thread's repository, matched without case", (await filter.inputValue()) === "sample/widgets");
  check("the filter lists each repository once", (await filter.locator("option").count()) === 3);
  check("the current repository shows its two items", (await panel.getByTestId("github-project-item").count()) === 2);
  check("items are grouped by status", (await panel.getByTestId("github-project-group").allInnerTexts()).map((text) => text.split("\n")[0]).join(",") === "Todo (1),In progress (1)");
  check("closed items are counted", (await panel.innerText()).includes("닫힌 항목 5개"));

  await filter.selectOption({ label: "전체" });
  check("all shows every item with the draft", (await panel.getByTestId("github-project-item").count()) === 4);

  const composer = page.getByTestId("composer-input");
  const waitForDraft = async (text) => {
    const deadline = Date.now() + 10_000;
    while (Date.now() < deadline) {
      if ((await composer.count()) > 0 && (await composer.inputValue()).includes(text)) return true;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    return false;
  };

  await filter.selectOption("sample/widgets");
  await panel.getByTestId("github-project-hand-off").first().click();
  check("hand-off fills the composer with the issue", await waitForDraft("Issue prompt for https://github.com/sample/widgets/issues/1"));
  check("hand-off picks the folder of the issue's repository", (await page.locator(".home-pickers").innerText()).includes("/work/example-app"));

  await filter.selectOption("sample/lib");
  await panel.getByTestId("github-project-hand-off").first().click();
  check("hand-off of another repository's issue fills the composer", await waitForDraft("Issue prompt for https://github.com/sample/lib/issues/3"));
  check("hand-off of another repository's issue leaves the folder to choose", !(await page.locator(".home-pickers").innerText()).includes("/work/example-app"));
  check("hand-off does not send the message", !gateway.receivedMethods.includes("turn/start"));

  await filter.selectOption({ label: "전체" });
  console.log("--- projects panel text ---");
  console.log(await panel.innerText());
  console.log("---------------------------");
  if (process.env.AKRA_QA_SCREENSHOT) {
    await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].showInactive());
    await page.screenshot({ path: process.env.AKRA_QA_SCREENSHOT, animations: "disabled" });
  }

  await panel.getByTestId("github-project-change").click();
  check("change shows the saved project", (await panel.getByLabel("Project").inputValue()) === "sample/1");
} catch (error) {
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  // 창을 닫으면 앱이 끝난다. app.close() 는 쓰지 않는다 (P2 기록 참고).
  const exited = new Promise((resolve) => app.process().once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  await gateway.close();
  await rm(userData, { recursive: true, force: true });
}

process.exit(failures.length === 0 ? 0 : 1);
