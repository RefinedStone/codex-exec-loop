// 화면 시험 (#2539): 이슈 목록은 가짜 저장소 커널이 준 표시와 순서를 그대로 사용한다.
// 숨긴 Electron 창과 임시 userData만 쓰며 실제 모델, GitHub, 브라우저는 호출하지 않는다.
// AKRA_QA_SCREENSHOT=<png>이면 목록·최대화·상세·정책 오류 증거를 같은 경로 접두사로 저장한다.
import { createRequire } from "node:module";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, extname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const issueScreenshotPath = process.env.AKRA_QA_ISSUES_SCREENSHOT ?? process.env.AKRA_QA_SCREENSHOT;

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

// 목록의 세 항목과 의도적으로 다른 전체 연결 보고다. 페이지 전환으로 합산하거나 축소하면 실패한다.
const globalReport = () => ({
  binding_id: binding.id,
  revision: automation.revision,
  policy_version: "fixture-policy-v1",
  policy_blocked: false,
  assumed_mode: "auto",
  total: 67,
  automatic_start: 17,
  human_approval: 23,
  excluded: 11,
  blocked: 5,
  deferred: 4,
  needs_info: 3,
  unknown: 2,
  untriaged: 2,
  stale: 1,
  observation_only: 51,
});
const SHARED_BODY = "현재 공유본만 표시합니다.\n\n## 요구 사항\n서버가 공유한 본문을 읽습니다.";
const UPDATED_SHARED_BODY = "새로 관찰한 현재 공유본입니다. 판정은 아직 갱신 전입니다.";
const triage = (issue) => ({
  revision: 4,
  policy_version: "fixture-policy-v1",
  observation_revision: 3,
  tracked_revision: 2,
  field_hash: { version: 1, sha256: "0".repeat(64) },
  basis: {
    version: 1,
    issue,
    source: "shared",
    disclosure_policy_version: "fixture-policy-v1",
    validation_policy_version: "fixture-validation-v1",
    requirements_sha256: "1".repeat(64),
    evidence_sha256: "2".repeat(64),
    pinned_source_commit: null,
  },
  stale: false,
  eligibility: "eligible",
  readiness: "needs_info",
  risk_class: "normal",
  effective_mode: "observe",
  reasons: [{
    code: "required_section_missing",
    summary: "서버 판정 문구: 수락 기준을 더 확인해야 합니다.",
    next: [{ step: "query", name: "fixture-do-not-convert-into-copy" }],
    next_summary: "서버 다음 행동: 작성자에게 완료 조건 두 가지를 확인하세요.",
  }],
  // 일반적인 단계 순서와 다르게 주어 renderer의 재정렬을 잡는다.
  gates: [
    { gate: "merge", action: "human", enabled: true, gate_revision: 4 },
    { gate: "start", action: "auto", enabled: true, gate_revision: 4 },
    { gate: "close", action: "human", enabled: false, gate_revision: 4 },
  ],
  observation: { assumed_mode: "auto", disposition: "human_approval", observation_only: true },
});
for (const value of [...firstItems, ...secondItems]) value.triage = triage(value.issue);
firstItems[2].current.content.shared_copy.body = SHARED_BODY;
let detailVariant = "shared";
let failNextDetailQuery = false;
let failNextPolicyQuery = false;
let policyBlocked = false;
const POLICY_WORKSPACE = "/work";
const policyFiles = [
  { scope: "user", label: "사용자 정책", path: "/home/fixture/.akra/issue-automation.toml", status: "readable", open_target: "user" },
  { scope: "project", label: "프로젝트 정책", path: "/work/.akra/issue-automation.toml", status: "readable", open_target: "project" },
];
const policySettings = [
  {
    key: "budget.max_parallel",
    label: "동시 실행 상한",
    value: "2",
    origins: [{ scope: "user", label: "서버 사용자 출처" }, { scope: "project", label: "서버 프로젝트 출처" }],
    resolution: "minimum",
    resolution_summary: "서버 해석: 사용자와 프로젝트가 정한 상한 중 작은 값입니다.",
  },
  {
    key: "actors.trusted_actors",
    label: "신뢰하는 사용자",
    value: "없음",
    origins: [{ scope: "project", label: "프로젝트에서 명시한 빈 목록" }],
    resolution: "intersection",
    resolution_summary: "서버 해석: 명시적 빈 목록을 보존했습니다.",
  },
];
const policyAnswer = () => policyBlocked ? {
  type: "issue_automation_policy",
  status: "policy_blocked",
  workspace: POLICY_WORKSPACE,
  blocked: {
    scope: "project",
    field: "budget.max_parallel",
    reason: "시험용 허용 범위를 벗어났습니다.",
    summary: "서버 정책 오류: 프로젝트의 동시 실행 상한을 확인하세요.",
    next_summary: "서버 복구 안내: 표시한 프로젝트 정책 파일을 수정하고 다시 조회하세요.",
    file_scopes: ["project"],
  },
  files: structuredClone(policyFiles),
} : {
  type: "issue_automation_policy",
  status: "ready",
  workspace: POLICY_WORKSPACE,
  version: "fixture-policy-v1",
  settings: structuredClone(policySettings),
  files: structuredClone(policyFiles),
};
const detailItem = (source) => {
  const value = structuredClone(source);
  if (detailVariant === "stale_triage") {
    value.current.content.shared_copy.body = UPDATED_SHARED_BODY;
    value.triage.stale = true;
    value.triage.gates = value.triage.gates.map((gate) => ({ ...gate, enabled: false }));
  } else if (detailVariant === "unknown") {
    value.current = { status: "unknown", reason: "provider_unavailable" };
    value.presentation.title = null;
  } else if (detailVariant === "withheld") {
    const { shared_copy: _copy, ...content } = value.current.content;
    value.current.content = { ...content, kind: "withheld", reason: "user_confirmation_required" };
    value.presentation.title = null;
  } else if (detailVariant === "legacy") {
    value.current = { status: "not_yet_observed" };
    value.presentation.title = null;
  }
  return value;
};

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
      if (params.query?.type === "issue_automation_policy") {
        if (failNextPolicyQuery) {
          failNextPolicyQuery = false;
          return { error: { code: -32603, message: "시험 정책을 잠시 읽지 못했습니다." } };
        }
        return { result: policyAnswer() };
      }
      if (params.query?.type === "issue") {
        if (failNextDetailQuery) {
          failNextDetailQuery = false;
          return { error: { code: -32603, message: "시험 상세를 잠시 읽지 못했습니다." } };
        }
        const source = [...firstItems, ...secondItems].find((value) => value.issue.issue_id === params.query.issue?.issue_id);
        return { result: source
          ? { type: "issue", status: "ready", revision: automation.revision, item: detailItem(source), report: globalReport() }
          : { type: "issue", status: "not_found", revision: automation.revision, report: globalReport() } };
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
            report: globalReport(),
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
    globalThis.issuesFixture = { opened: [], externalRequests: [], policyRequests: [], rejectPolicyFile: false };
    shell.openExternal = async (url) => { globalThis.issuesFixture.opened.push(url); };
    // 파일 열기의 좁은 intent만 관찰한다. 실제 경로 허용/거부는 main의 별도 시험이 맡는다.
    ipcMain.removeHandler("issueAutomationPolicy:openFile");
    ipcMain.handle("issueAutomationPolicy:openFile", (_event, request) => {
      globalThis.issuesFixture.policyRequests.push(request);
      if (globalThis.issuesFixture.rejectPolicyFile) throw new Error("시험 정책 파일을 열지 못했습니다.");
    });

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

  const captureVariant = async (suffix) => {
    const path = issueScreenshotPath;
    if (!path) return;
    const extension = extname(path);
    await capture(`${extension ? path.slice(0, -extension.length) : path}-${suffix}${extension || ".png"}`);
  };

  if (issueScreenshotPath) await capture(issueScreenshotPath);
  const rightPanel = page.getByTestId("right-panel");
  const beforeMaximize = await rightPanel.boundingBox();
  await page.getByTestId("issues-maximize").click();
  check("maximizing fills the workspace panel", await until(async () => (await rightPanel.getAttribute("data-maximized")) === "true"));
  const maximized = await rightPanel.boundingBox();
  check("the maximized panel is wider than the original panel", beforeMaximize !== null && maximized !== null && maximized.width > beforeMaximize.width);
  if (issueScreenshotPath) {
    const path = issueScreenshotPath;
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
  const detail = panel.getByTestId("issues-detail");
  check("the title opens the current shared copy inside the issue panel", await until(async () => (await detail.getByTestId("issues-shared-copy").count()) === 1 && (await detail.getByTestId("issues-shared-copy").innerText()).includes("현재 공유본만 표시합니다.")));
  const detailRequests = () => requests.filter((request) => request.method === "akra/kernel/query" && request.params.query?.type === "issue");
  check("the detail query sends the complete server issue reference", JSON.stringify(detailRequests()[0]?.params.query.issue) === JSON.stringify(firstItems[2].issue));
  check("selecting a title does not open an external browser", (await app.evaluate(() => globalThis.issuesFixture.opened)).length === 0);
  check("detail reasons and next actions are exactly the server's sentences", (await detail.getByTestId("issues-triage-summary").innerText()) === firstItems[2].triage.reasons[0].summary && (await detail.getByTestId("issues-triage-next").innerText()) === firstItems[2].triage.reasons[0].next_summary);
  check("the gate list preserves server order and enabled flags", JSON.stringify(await detail.getByTestId("issues-triage-gate").evaluateAll((elements) => elements.map((element) => ({ gate: element.dataset.gate, enabled: element.dataset.enabled })))) === JSON.stringify([{ gate: "merge", enabled: "true" }, { gate: "start", enabled: "true" }, { gate: "close", enabled: "false" }]));
  check("a narrow detail view replaces the list and offers a way back", (await rows.count()) === 0 && await detail.getByTestId("issues-detail-back").isVisible());
  await captureVariant("detail");
  await page.getByTestId("issues-maximize").click();
  check("the existing maximize action also applies to the detail", await until(async () => (await rightPanel.getAttribute("data-maximized")) === "true") && await detail.isVisible());
  await captureVariant("detail-maximized");
  await page.getByTestId("issues-restore").click();

  detailVariant = "stale_triage";
  await panel.getByTestId("issues-refresh").click();
  check("a current shared copy remains visible when only its saved triage is stale", await until(async () => (await detail.getByTestId("issues-triage-stale").count()) === 1 && (await detail.getByTestId("issues-shared-copy").innerText()) === UPDATED_SHARED_BODY));
  check("a stale triage keeps the server's disabled gate flags", (await detail.getByTestId("issues-triage-gate").evaluateAll((elements) => elements.every((element) => element.dataset.enabled === "false"))));
  for (const [variant, expectedMessage] of [
    ["unknown", "현재 공유본을 확인하지 못했습니다."],
    ["withheld", "현재 정책에서 공유하지 않은 본문입니다."],
    ["legacy", "아직 현재 공유본을 관찰하지 않았습니다."],
  ]) {
    detailVariant = variant;
    await panel.getByTestId("issues-refresh").click();
    check(`${variant} detail does not reuse a previous shared body`, await until(async () => (await detail.getByTestId("issues-shared-unavailable").count()) === 1 && (await detail.getByTestId("issues-shared-unavailable").innerText()) === expectedMessage) && (await detail.getByTestId("issues-shared-copy").count()) === 0 && !(await detail.innerText()).includes(UPDATED_SHARED_BODY));
  }
  failNextDetailQuery = true;
  await panel.getByTestId("issues-refresh").click();
  check("a detail query failure is visible without recovering an old body", await until(async () => (await detail.getByTestId("issues-detail-error").count()) === 1) && (await detail.getByTestId("issues-shared-copy").count()) === 0);
  detailVariant = "shared";
  await panel.getByTestId("issues-refresh").click();
  check("refresh recovers the selected detail", await until(async () => (await detail.getByTestId("issues-shared-copy").count()) === 1 && (await detail.getByTestId("issues-detail-error").count()) === 0));
  await detail.getByTestId("issues-detail-back").click();
  check("returning from detail restores the server's list order", await until(async () => (await rows.count()) === 3 && JSON.stringify(await readRows()) === JSON.stringify(expected)));

  await panel.getByTestId("issues-report-tab").click();
  const report = panel.getByTestId("issues-report");
  const readPrimaryCounts = async () => ({
    automatic_start: await report.getByTestId("issues-report-automatic-start").innerText(),
    human_approval: await report.getByTestId("issues-report-human-approval").innerText(),
    excluded: await report.getByTestId("issues-report-excluded").innerText(),
    total: await report.getByTestId("issues-report-total").innerText(),
  });
  const expectedCounts = { automatic_start: "17", human_approval: "23", excluded: "11", total: "67" };
  check("the report uses whole-binding server counts rather than its three visible rows", JSON.stringify(await readPrimaryCounts()) === JSON.stringify(expectedCounts));
  check("the report names its current tracked binding and all-page scope", (await report.getByTestId("issues-report-binding").innerText()) === binding.id && (await report.getByTestId("issues-report-scope").innerText()).includes("전체 이슈 · 모든 페이지"));
  await captureVariant("report");
  await panel.getByTestId("issues-list-tab").click();


  await panel.getByTestId("issues-policy-tab").click();
  const policy = panel.getByTestId("issues-policy");
  const settings = policy.getByTestId("issues-policy-setting");
  check("the policy view displays the server's effective values", await until(async () => (await settings.count()) === policySettings.length) && JSON.stringify(await policy.getByTestId("issues-policy-value").allInnerTexts()) === JSON.stringify(policySettings.map((setting) => setting.value)));
  const firstSetting = settings.first();
  check("policy provenance and resolution are not inferred in the renderer", JSON.stringify(await firstSetting.locator("[data-scope]").allInnerTexts()) === JSON.stringify(policySettings[0].origins.map((origin) => origin.label)) && (await firstSetting.getByTestId("issues-policy-resolution").innerText()) === policySettings[0].resolution_summary);
  check("an explicitly empty policy value retains its server provenance", (await settings.nth(1).getByTestId("issues-policy-value").innerText()) === "없음" && (await settings.nth(1).getByTestId("issues-policy-origins").innerText()).includes("프로젝트에서 명시한 빈 목록"));
  check("policy file locations are exactly the returned paths", JSON.stringify(await policy.getByTestId("issues-policy-path").allInnerTexts()) === JSON.stringify(policyFiles.map((file) => file.path)));
  const projectFile = policy.getByTestId("issues-policy-file").filter({ hasText: "프로젝트 정책" });
  await app.evaluate(() => { globalThis.issuesFixture.rejectPolicyFile = true; });
  await projectFile.getByTestId("issues-policy-file-open").click();
  check("a rejected asynchronous file open shows its error", await until(async () => (await policy.getByTestId("issues-policy-open-error").count()) === 1 && (await policy.getByTestId("issues-policy-open-error").innerText()).includes("시험 정책 파일을 열지 못했습니다.")));
  check("file open uses the server workspace, scope and expected path with a local host", JSON.stringify((await app.evaluate(() => globalThis.issuesFixture.policyRequests))[0]) === JSON.stringify({ workspace: POLICY_WORKSPACE, scope: "project", expectedPath: policyFiles[1].path, host: { kind: "local" } }) && POLICY_WORKSPACE !== WORKSPACE);
  await app.evaluate(() => { globalThis.issuesFixture.rejectPolicyFile = false; });
  await projectFile.getByTestId("issues-policy-file-open").click();
  check("a successful file-open intent clears the previous error", await until(async () => (await app.evaluate(() => globalThis.issuesFixture.policyRequests)).length === 2 && (await policy.getByTestId("issues-policy-open-error").count()) === 0));

  policyBlocked = true;
  await panel.getByTestId("issues-refresh").click();
  check("a blocked policy shows the exact cause and next step", await until(async () => (await policy.getByTestId("issues-policy-blocked").count()) === 1) && (await policy.getByTestId("issues-policy-blocked-summary").innerText()) === policyAnswer().blocked.summary && (await policy.getByTestId("issues-policy-blocked-reason").innerText()) === policyAnswer().blocked.reason && (await policy.getByTestId("issues-policy-blocked-next").innerText()) === policyAnswer().blocked.next_summary);
  check("the blocked policy keeps the exact project file location", (await projectFile.getByTestId("issues-policy-path").innerText()) === policyFiles[1].path && (await projectFile.getAttribute("data-blocked")) === "true" && (await settings.count()) === 0);
  await captureVariant("policy-blocked");
  failNextPolicyQuery = true;
  await panel.getByTestId("issues-refresh").click();
  check("a failed policy refresh does not present earlier values or causes as current", await until(async () => (await policy.getByTestId("issues-policy-error").count()) === 1) && (await policy.getByTestId("issues-policy-blocked").count()) === 0 && (await settings.count()) === 0);
  policyBlocked = false;
  await panel.getByTestId("issues-refresh").click();
  check("policy refresh recovers independently from the issue list", await until(async () => (await settings.count()) === policySettings.length && (await policy.getByTestId("issues-policy-error").count()) === 0));
  await panel.getByTestId("issues-list-tab").click();

  await rows.first().getByTestId("issues-external").click();

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
  await panel.getByTestId("issues-report-tab").click();
  check("the next page keeps the same whole-binding counts", JSON.stringify(await readPrimaryCounts()) === JSON.stringify(expectedCounts));
  await panel.getByTestId("issues-list-tab").click();
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
