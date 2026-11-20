// 화면 시험 (#2539): 이슈 목록은 가짜 저장소 커널이 준 표시와 순서를 그대로 사용한다.
// 숨긴 Electron 창과 임시 userData만 쓰며 실제 모델, GitHub, 브라우저는 호출하지 않는다.
// AKRA_QA_SCREENSHOT=<png>이면 정상 세 행과 최대화한 패널을 각각 저장한다.
import { createRequire } from "node:module";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, extname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);

const WORKSPACE = "/work/example-app";
const NEXT_PAGE = "fixture-next-page";
const LAST_SUCCESS = Date.parse("2026-11-20T08:05:00Z");
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const requests = [];
const binding = {
  id: "fixture-origin-binding",
  provider: "github",
  server_id: "github.com",
  repository_id: "R_fixture_widgets",
  project_scope: { workspace_dir: WORKSPACE },
};
const automation = {
  status: "ready",
  mode: "off",
  epoch: 1,
  revision: 7,
  binding_id: binding.id,
  binding,
  last_attempt_at_ms: LAST_SUCCESS,
  last_success_at_ms: LAST_SUCCESS,
  last_failure: null,
  consecutive_failures: 0,
  collecting: false,
  next_poll_after_ms: null,
};

// 모두 시험용 공유본이다. 번호나 제목으로 정렬하면 서버가 준 그룹 순서와 달라진다.
const item = (number, title, state, statusLabel, reason) => ({
  issue: {
    provider: binding.provider,
    server_id: binding.server_id,
    repository_id: binding.repository_id,
    issue_id: `I_fixture_${number}`,
    project_scope: binding.project_scope,
  },
  binding_id: binding.id,
  tracked: { state, held: false, stale: false, unknown: false },
  current: {
    status: "observed",
    remote_state: "open",
    labels: { completeness: "complete", items: [] },
    assignees: { completeness: "complete", items: [] },
    author: null,
    label_actors: { completeness: "complete", items: [] },
    linked_pull_request: "absent",
    content: {
      kind: "shared",
      basis: {
        field_hash: { version: 1, sha256: "0".repeat(64) },
        provider_updated_at: "2026-11-20T08:00:00Z",
      },
      shared_copy: { title, body: "화면 시험용 공유 설명입니다." },
      policy_version: "fixture-policy-v1",
    },
  },
  presentation: {
    number,
    url: `https://github.com/sample/widgets/issues/${number}`,
    title,
    status: { code: state, label: statusLabel },
    reason: { code: "workflow", summary: reason },
  },
});
const firstItems = [
  item(31, "가 · 공유 목록 유지", "discovered", "발견됨", "이슈를 수집했습니다."),
  item(4, "나 · 결정 대기 표시", "awaiting_decision", "결정 대기", "사용자의 결정을 기다립니다."),
  item(18, "다 · 서버 순서 유지", "ready", "준비됨", "실행할 준비가 되었습니다."),
];
const secondItems = [
  item(77, "두 번째 페이지의 공유 이슈", "discovered", "발견됨", "이슈를 수집했습니다."),
];
const serverOrder = [2, 0, 1];
const expectedRows = serverOrder.map((index) => firstItems[index].presentation);
const stateEvent = () => ({ type: "issues", ...structuredClone(automation) });
let failNextIssuesQuery = false;

const kernel = {
  kernel: "fixture-issues-repository",
  queries: {},
  onRequest(method, params) {
    if (method === "akra/kernel/query") {
      if (params.query?.type === "issue_automation") {
        return { result: { type: "issue_automation", ...structuredClone(automation) } };
      }
      if (params.query?.type === "issues") {
        if (failNextIssuesQuery) {
          failNextIssuesQuery = false;
          return { error: { code: -32603, message: "시험 목록을 잠시 읽지 못했습니다." } };
        }
        const after = params.query.after ?? null;
        if (after !== null && after !== NEXT_PAGE) {
          return { error: { code: -32602, message: "시험 커서가 일치하지 않습니다." } };
        }
        const next = after === NEXT_PAGE;
        return {
          result: {
            type: "issues",
            status: "ready",
            revision: automation.revision,
            items: structuredClone(next ? secondItems : firstItems),
            groups: [{ id: "collection", label: "수집한 이슈", item_indices: next ? [0] : serverOrder }],
            next_after: next ? null : NEXT_PAGE,
          },
        };
      }
    }
    if (method === "akra/kernel/command") {
      const command = params.command;
      if (command?.type === "set_issue_automation_mode") {
        const previous = automation.mode;
        automation.mode = command.mode;
        automation.epoch += 1;
        automation.revision += 1;
        automation.next_poll_after_ms = command.mode === "observe" ? 300_000 : null;
        return {
          result: {
            processed: true,
            result: {
              type: "issue_automation_mode_changed",
              previous,
              mode: command.mode,
              epoch: automation.epoch,
              revision: automation.revision,
              changed: previous !== command.mode,
            },
          },
          events: [stateEvent()],
        };
      }
      if (command?.type === "sync_issues") {
        automation.last_attempt_at_ms = LAST_SUCCESS + 60_000;
        automation.revision += 1;
        return { result: { processed: true }, events: [stateEvent()] };
      }
      return { error: { code: -32602, message: "이슈 화면은 다른 커널 명령을 보내지 않습니다." } };
    }
    return undefined;
  },
};

const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(
  endpoint,
  [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING],
  { kernel, onRequest: (method, params) => requests.push({ method, params: structuredClone(params) }) },
);
const userData = await mkdtemp(join(tmpdir(), "akra-issues-panel-ui-"));
const env = {
  ...process.env,
  AKRA_GATEWAY_ENDPOINT: endpoint,
  AKRA_DESKTOP_HIDDEN: "1",
  AKRA_DESKTOP_USER_DATA: userData,
};
delete env.ELECTRON_RUN_AS_NODE;

const failures = [];
const check = (name, ok) => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}`);
  if (!ok) failures.push(name);
};
const until = async (condition, timeout = 10_000) => {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await condition()) return true;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  return false;
};
const kernelCommands = (type) => requests.filter(
  (request) => request.method === "akra/kernel/command" && request.params.command?.type === type,
);
const countRequests = (method) => requests.filter((request) => request.method === method).length;
const issueQueries = () => requests.filter(
  (request) => request.method === "akra/kernel/query" && request.params.query?.type === "issues",
);
let app;
try {
  app = await electron.launch({
    executablePath: electronPath,
    args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])],
    env,
  });
  // main의 URL 검사는 그대로 둔다. OS 브라우저 실행만 관찰 가능한 가짜로 바꾼다.
  await app.evaluate(({ ipcMain, shell }) => {
    globalThis.issuesFixture = { opened: [], externalRequests: [] };
    shell.openExternal = async (url) => { globalThis.issuesFixture.opened.push(url); };
    ipcMain.on("akra:open-external", (_event, url) => {
      globalThis.issuesFixture.externalRequests.push(url);
    });
  });
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("issues-toggle").click();
  const panel = page.getByTestId("issues-panel");
  await panel.waitFor({ timeout: 10_000 });
  const rows = panel.getByTestId("issues-row");
  const mode = panel.getByTestId("issues-mode");
  check("the server's first page renders all three issues", await until(async () => (await rows.count()) === 3 && (await mode.inputValue()) === "off" && await mode.isEnabled()));
  const readRows = async () => Promise.all((await rows.all()).map(async (row) => ({
    number: (await row.getByTestId("issues-number").innerText()).trim(),
    title: (await row.getByTestId("issues-title").innerText()).trim(),
    status: (await row.getByTestId("issues-status").innerText()).trim(),
    reason: (await row.getByTestId("issues-reason").innerText()).trim(),
  })));
  const expected = expectedRows.map((value) => ({
    number: `#${value.number}`,
    title: value.title,
    status: value.status.label,
    reason: value.reason.summary,
  }));
  check("numbers, titles, states and reasons follow the server's group order", JSON.stringify(await readRows()) === JSON.stringify(expected));
  check("the server's group label is displayed", (await panel.getByTestId("issues-group").count()) === 1 && (await panel.getByTestId("issues-group").getAttribute("aria-label")) === "수집한 이슈" && (await panel.getByTestId("issues-group-title").innerText()).startsWith("수집한 이슈"));
  check("the saved Off mode is displayed without writing it again", (await mode.inputValue()) === "off" && kernelCommands("set_issue_automation_mode").length === 0);
  check("manual sync is disabled while automation is Off", await panel.getByTestId("issues-sync").isDisabled());
  check("the first issues query has no cursor", (issueQueries()[0]?.params.query.after ?? null) === null);

  await mode.selectOption("observe");
  check("changing mode sends one Observe command", await until(() => kernelCommands("set_issue_automation_mode").length === 1 && automation.mode === "observe"));
  const modeRequest = kernelCommands("set_issue_automation_mode")[0]?.params;
  check("a mode mutation carries a UUID idempotency key", typeof modeRequest?.idempotency_key === "string" && UUID.test(modeRequest.idempotency_key));
  check("the mode command contains only the requested mode", modeRequest?.command.type === "set_issue_automation_mode" && modeRequest.command.mode === "observe" && Object.keys(modeRequest.command).sort().join("|") === "mode|type");
  await panel.getByTestId("issues-sync").click();
  check("manual sync sends one read command", await until(() => kernelCommands("sync_issues").length === 1));
  const syncRequest = kernelCommands("sync_issues")[0]?.params;
  check("manual sync has no mutation idempotency key", syncRequest?.idempotency_key == null && JSON.stringify(syncRequest?.command) === JSON.stringify({ type: "sync_issues" }));
  const lastSuccess = panel.getByTestId("issues-last-success");
  await lastSuccess.waitFor({ timeout: 10_000 });
  const successText = (await lastSuccess.innerText()).trim();
  const expectedSuccess = await page.evaluate((at) => new Intl.DateTimeFormat("ko-KR", { dateStyle: "medium", timeStyle: "short" }).format(new Date(at)), LAST_SUCCESS);
  check("the last successful collection uses the server timestamp", successText === expectedSuccess);
  check("a successful collection does not display an error", (await panel.getByTestId("issues-error").count()) === 0);
  check("mode and sync retain the server's three-row order", await until(async () => (await panel.getByTestId("issues-sync").isEnabled()) && JSON.stringify(await readRows()) === JSON.stringify(expected)));

  // 숨긴 Windows 창에서는 page.screenshot이 멈출 수 있어 main에서 직접 캡처한다.
  const capture = async (path) => {
    const png = () => app.evaluate(async ({ BrowserWindow }) => (
      await BrowserWindow.getAllWindows()[0].webContents.capturePage()
    ).toPNG().toString("base64"));
    await png();
    await page.waitForTimeout(150);
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, Buffer.from(await png(), "base64"));
  };
  if (process.env.AKRA_QA_SCREENSHOT) await capture(process.env.AKRA_QA_SCREENSHOT);
  const rightPanel = page.getByTestId("right-panel");
  const beforeMaximize = await rightPanel.boundingBox();
  await page.getByTestId("issues-maximize").click();
  check("maximizing fills the workspace panel", await until(async () => (await rightPanel.getAttribute("data-maximized")) === "true"));
  const maximized = await rightPanel.boundingBox();
  check("the maximized panel is wider than the original panel", beforeMaximize !== null && maximized !== null && maximized.width > beforeMaximize.width);
  if (process.env.AKRA_QA_SCREENSHOT) {
    const path = process.env.AKRA_QA_SCREENSHOT;
    const extension = extname(path);
    await capture(`${extension ? path.slice(0, -extension.length) : path}-maximized${extension || ".png"}`);
  }
  await page.getByTestId("issues-restore").click();
  check("restoring returns the panel to its prior width", await until(async () => {
    const bounds = await rightPanel.boundingBox();
    return (await rightPanel.getAttribute("data-maximized")) === "false" && bounds !== null
      && beforeMaximize !== null && Math.abs(bounds.width - beforeMaximize.width) <= 1;
  }));

  await rows.first().getByTestId("issues-title").click();
  check("a permitted GitHub URL reaches the OS-opening boundary", await until(async () => (
    await app.evaluate(() => globalThis.issuesFixture.opened)
  ).includes(expectedRows[0].url)));
  const maliciousUrls = [
    "javascript:alert('fixture')",
    "file:///C:/fixture/issue.txt",
    "https://github.com.attacker.invalid/sample/widgets/issues/18",
    "https://github.com@attacker.invalid/sample/widgets/issues/18",
  ];
  // renderer의 링크 필터를 우회해도 main의 기존 검사가 차단해야 한다.
  await page.evaluate((urls) => { for (const url of urls) window.akra.github.openExternal(url); }, maliciousUrls);
  check("malicious URLs reached the real main IPC handler", await until(async () => (
    await app.evaluate(() => globalThis.issuesFixture.externalRequests)
  ).filter((url) => maliciousUrls.includes(url)).length === maliciousUrls.length));

  await panel.getByTestId("issues-next-page").click();
  check("the next page replaces the previous page", await until(async () => (await rows.count()) === 1 && (await rows.first().getByTestId("issues-title").innerText()) === secondItems[0].presentation.title));
  check("pagination sends the exact server cursor", issueQueries().some((request) => request.params.query.after === NEXT_PAGE));
  check("the bounded page size is sent to the kernel", issueQueries().every((request) => request.params.query.limit === 50));
  check("the second page has no next-page action", await panel.getByTestId("issues-next-page").isDisabled());
  check("the page counter advances", (await panel.getByTestId("issues-page-count").innerText()).includes("2"));
  await panel.getByTestId("issues-previous-page").click();
  check("the previous page restores the original server order", await until(async () => (await rows.count()) === 3 && JSON.stringify(await readRows()) === JSON.stringify(expected)));
  check("the first page has no previous-page action", await panel.getByTestId("issues-previous-page").isDisabled());

  automation.last_failure = { kind: "source", reason: "provider_unavailable" };
  automation.consecutive_failures = 1;
  automation.revision += 1;
  gateway.sendKernelEvent(stateEvent());
  check("a collection failure is visible after a pushed state update", await until(async () => (
    await panel.getByTestId("issues-error").count()
  ) > 0 && (await panel.getByTestId("issues-error").innerText()).includes("수집 오류: 이슈 공급자에 연결하지 못했습니다.")));
  check("a collection failure retains the last successful time", (await lastSuccess.innerText()).trim() === successText);
  automation.last_failure = null;
  automation.consecutive_failures = 0;
  automation.revision += 1;
  gateway.sendKernelEvent(stateEvent());
  check("a later successful state clears the collection error", await until(async () => (await panel.getByTestId("issues-error").count()) === 0 && (await rows.count()) === 3 && await panel.getByTestId("issues-refresh").isEnabled()));

  failNextIssuesQuery = true;
  await panel.getByTestId("issues-refresh").click();
  check("a failed page read is visible", await until(async () => (await panel.getByTestId("issues-error").count()) > 0));
  await panel.getByTestId("issues-refresh").click();
  check("refresh recovers the page without a new mode command", await until(async () => (await rows.count()) === 3 && (await panel.getByTestId("issues-error").count()) === 0) && kernelCommands("set_issue_automation_mode").length === 1);

  const attachments = countRequests("akra/kernel/attach");
  const reads = issueQueries().length;
  gateway.disconnectClients();
  check("a reconnect attaches and reads the issue list again", await until(async () => (
    countRequests("akra/kernel/attach") > attachments && issueQueries().length > reads
      && (await rows.count()) === 3 && (await mode.inputValue()) === "observe"
  ), 20_000));
  check("a reconnect does not replay mode or sync commands", kernelCommands("set_issue_automation_mode").length === 1 && kernelCommands("sync_issues").length === 1);
  check("the main process rejected every malicious URL", JSON.stringify(await app.evaluate(() => globalThis.issuesFixture.opened)) === JSON.stringify([expectedRows[0].url]));

  console.log("--- issues panel text ---");
  console.log(await panel.innerText());
  console.log("-------------------------");
  const detaches = countRequests("akra/kernel/detach");
  await page.getByTestId("issues-toggle").click();
  await panel.waitFor({ state: "detached", timeout: 10_000 });
  check("closing the issue panel detaches its kernel subscription", await until(() => countRequests("akra/kernel/detach") > detaches));
  check("the issues tab sends no planning commands or planning queries", requests.every((request) => {
    if (request.method === "akra/kernel/command") return ["set_issue_automation_mode", "sync_issues"].includes(request.params.command?.type);
    if (request.method === "akra/kernel/query") return !["planning", "parallel"].includes(request.params.query?.type);
    return true;
  }));
  check("the issue panel never starts or steers a model turn", !gateway.receivedMethods.includes("turn/start") && !gateway.receivedMethods.includes("turn/steer"));
} catch (error) {
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  // 현재 시험이 연 창만 닫는다. app.close()로 Electron을 강제 종료하지 않는다.
  if (app && app.process().exitCode === null) {
    const exited = new Promise((resolve) => app.process().once("exit", resolve));
    for (const window of app.windows()) await window.close();
    let timer;
    const closed = await Promise.race([
      exited.then(() => true),
      new Promise((resolve) => { timer = setTimeout(() => resolve(false), 10_000); }),
    ]);
    clearTimeout(timer);
    if (!closed) {
      check("the hidden test application exits after closing its windows", false);
      app.process().kill();
    }
  }
  await gateway.close();
  await rm(userData, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 });
}

process.exit(failures.length === 0 ? 0 : 1);
