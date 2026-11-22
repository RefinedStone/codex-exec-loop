// #2554: 저장된 실행 표시값과 읽기 동작만 fake Gateway로 검증한다. 실제 모델·공급자를 호출하지 않는다.
// AKRA_QA_EXECUTION_SCREENSHOT=<png>이면 compact/maximized/timeline/unavailable 증거를 저장한다.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, extname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);
const screenshotPath = process.env.AKRA_QA_EXECUTION_SCREENSHOT;
const workspace = "/work/example-app";
const issue = { provider: "github", server_id: "github.com", repository_id: "R_execution", issue_id: "I_execution", project_scope: { workspace_dir: workspace } };
const binding = { id: "execution-binding", provider: issue.provider, server_id: issue.server_id, repository_id: issue.repository_id, project_scope: issue.project_scope };
const issueUrl = "https://github.com/sample/widgets/issues/2554#issuecomment-83";
const prUrl = "https://github.com/sample/widgets/pull/81?expand=1#discussion-fixture";
const targetThread = "00000000-2554-4000-8000-000000000001";
const currentBody = "실행 추적 시험의 처음 대화입니다.";
const targetBody = "서버가 연결한 작업 대화로 이동했습니다.";
const resumeTemplate = fake.loadReplayTable(fake.INTERRUPTED_TURN_RECORDING).get("thread/resume").result;
const initialThread = resumeTemplate.thread.id;
assert.notEqual(targetThread, initialThread);
const requests = [];
let revision = 7;
let eventSequence = 200;
let detailStatus = "ready";
let failNextDetail = false;
let heldDetail = null;
let group = { id: "running", label: "실행 중" };
let conversation = { kind: "thread", thread_id: initialThread };
let execution = {
  status: "ready", work_order_id: "work-order-execution", revision: 41,
  state: "running", state_label: "서버 실행 중 · 검증 준비",
  execution_attempts: 2, execution_attempt_limit: 3, verification_attempts: 4,
  review_pull_request_url: null, terminal_class: null, recovery: null,
};
let feedback = { revision: 2, publication_attempts: 5, external_state: "open", reason: null, integration: null };
let timeline = {
  status: "ready",
  // 시각은 의도적으로 역순이다. renderer가 서버의 sequence 순서를 다시 정렬하면 실패한다.
  entries: Array.from({ length: 20 }, (_, index) => ({
    sequence: index + 101,
    recorded_at: new Date(Date.parse("2026-11-22T09:20:00Z") - index * 60_000).toISOString(),
    kind: index % 2 === 0 ? "issue_execution_progressed" : "issue_feedback_posted",
    summary: "서버 기록 " + (index + 101) + " · 저장된 진행 사실",
  })),
  has_older: true,
};
const recovery = {
  reason: "contract_recovery",
  summary: "서버 종료 이유: 통합 뒤 계약 검증에서 예상한 응답을 확인하지 못했습니다.",
  next_summary: "서버 다음 행동: 통합된 변경과 계약 검증 결과를 사람이 확인해 주세요.",
  requires_human: true,
};
const integration = { record_id: "integration-fixture", source_sha: "a".repeat(40), integration_sha: "b".repeat(40), pull_request_url: prUrl };
const item = () => ({
  issue, binding_id: binding.id,
  tracked: { state: group.id, held: false, stale: false, unknown: false },
  work: { work_order_id: "work-order-execution", task_ids: ["task-execution"], state: group.id },
  current: {
    status: "observed", remote_state: "open",
    labels: { completeness: "complete", items: [] }, assignees: { completeness: "complete", items: [] },
    author: null, label_actors: { completeness: "complete", items: [] }, linked_pull_request: "absent",
    content: {
      kind: "shared", basis: { field_hash: { version: 1, sha256: "0".repeat(64) }, provider_updated_at: "2026-11-22T09:00:00Z" },
      shared_copy: { title: "통합과 외부 상태를 함께 확인하기", body: "## 목표\n저장된 실행 경로와 서버의 다음 행동을 확인합니다." },
      policy_version: "fixture-policy",
    },
  },
  presentation: {
    number: 2554, title: "통합과 외부 상태를 함께 확인하기", url: issueUrl,
    status: { code: group.id, label: group.label }, reason: { code: "workflow", summary: "서버가 반환한 대표 상태와 진행 이유입니다." },
  },
});
const automation = (type = "issue_automation") => ({
  type, status: "ready", mode: "observe", epoch: 1, revision, binding_id: binding.id, binding,
  collecting: false, consecutive_failures: 0, last_success_at_ms: Date.parse("2026-11-22T09:00:00Z"),
});
const detail = () => detailStatus === "unavailable" ? { type: "issue", status: "unavailable" } : ({
  type: "issue", status: "ready", revision, item: item(), open_decisions: [], decision_history: [],
  control: { tracked: true, held: false, taken_over: false, cancelled: false, authorization_epoch: 1 },
  actions: [], conversation: structuredClone(conversation),
  execution: structuredClone(execution), feedback: structuredClone(feedback), timeline: structuredClone(timeline),
});
const kernel = {
  kernel: "execution-tracking-fixture", queries: {},
  onRequest(method, params) {
    if (method === "akra/kernel/query") {
      if (params.query?.type === "issue_automation") return { result: automation() };
      if (params.query?.type === "issues") return { result: { type: "issues", status: "ready", revision, items: [item()], groups: [{ ...group, item_indices: [0] }], next_after: null } };
      if (params.query?.type === "issue_automation_policy") return { result: { type: "issue_automation_policy", status: "ready", workspace, version: "fixture-policy", files: [], settings: [] } };
      if (params.query?.type === "issue") {
        assert.deepEqual(params.query.issue, issue);
        if (failNextDetail) {
          failNextDetail = false;
          return { error: { code: -32013, message: "시험 상세 조회 실패" } };
        }
        const reply = { result: detail() };
        if (!heldDetail) return reply;
        const held = heldDetail;
        heldDetail = null;
        held.requested = true;
        return { ...reply, replyAfter: held.replyAfter };
      }
    }
    if (method === "akra/kernel/command" || method === "akra/kernel/command_confirmed") {
      return { result: { processed: false, failure: "실행 추적 시험은 명령을 허용하지 않습니다." } };
    }
    return undefined;
  },
};
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  kernel,
  onRequest: (method, params) => requests.push({ method, params: structuredClone(params) }),
  readThread(params) {
    assert.ok(params.threadId === initialThread || params.threadId === targetThread);
    const reply = structuredClone(resumeTemplate);
    const text = params.threadId === targetThread ? targetBody : currentBody;
    reply.thread.id = params.threadId;
    reply.thread.sessionId = params.threadId;
    reply.thread.preview = text;
    reply.thread.name = params.threadId === targetThread ? "연결된 작업 대화" : "처음 대화";
    reply.thread.turns = [{
      id: "fixture-turn-" + params.threadId, status: "completed", error: null,
      startedAt: 1795341600, completedAt: 1795341601, durationMs: 1000, itemsView: "full",
      items: [{ type: "agentMessage", id: "fixture-message-" + params.threadId, text, phase: "final_answer", memoryCitation: null, delivery: null, questions: null }],
    }];
    reply.turnsBackwardsCursor = null;
    reply.itemsBackwardsCursor = null;
    return reply;
  },
});
const userData = await mkdtemp(join(tmpdir(), "akra-issue-execution-ui-"));
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;
const until = async (condition, description) => {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    if (await condition()) return;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error("execution tracking fixture did not reach " + description);
};
const issueQueries = () => requests.filter((request) => request.method === "akra/kernel/query" && request.params.query?.type === "issue");
const writes = () => requests.filter((request) => ["akra/kernel/command", "akra/kernel/command_confirmed", "turn/start", "turn/steer", "thread/start"].includes(request.method));
const emit = (kind, journalRevision = 2) => {
  const common = { kind, event_id: "fixture-event-" + (++eventSequence), issue, revision: journalRevision };
  const event = kind === "issue_execution_progressed"
    ? { ...common, work_order_id: "work-order-execution", progress: { revision: execution.revision, state: execution.state, attempts: { execution: execution.execution_attempts, verification: execution.verification_attempts, publication: 0 }, attempts_exhausted: false } }
    : kind === "issue_integrated" ? { ...common, work_order_id: "work-order-execution", integration }
      : kind === "issue_feedback_posted" ? { ...common, intent_id: "intent-feedback", actor_id: "actor-fixture", action: "comment_updated", comment_id: "comment-fixture" }
        : { ...common, origin: "confirmed_feedback", intent_id: "intent-close", actor_id: "actor-fixture", closure_event_id: "closed-fixture" };
  gateway.sendKernelEvent({ type: "recorded", sequence: eventSequence, recorded_at: "2026-11-22T10:00:00Z", event });
};
let app;
let appProcess;
let page;
const capture = async (suffix) => {
  if (!screenshotPath || !app) return;
  const extension = extname(screenshotPath);
  const path = (extension ? screenshotPath.slice(0, -extension.length) : screenshotPath) + "-" + suffix + (extension || ".png");
  // 숨긴 Windows 창의 page.screenshot 대신 main의 capturePage로 현재 프레임을 받는다.
  const png = () => app.evaluate(async ({ BrowserWindow }) => (await BrowserWindow.getAllWindows()[0].webContents.capturePage()).toPNG().toString("base64"));
  await png();
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, Buffer.from(await png(), "base64"));
};
try {
  app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env, cwd: desktopDir });
  appProcess = app.process();
  await app.evaluate(({ shell, dialog }) => {
    globalThis.executionFixture = { opened: [], dialogs: [] };
    shell.openExternal = async (url) => { globalThis.executionFixture.opened.push(url); };
    dialog.showMessageBox = async (_owner, options) => {
      globalThis.executionFixture.dialogs.push(options);
      return { response: 1, checkboxChecked: false };
    };
  });
  page = await app.firstWindow();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  await page.getByText(currentBody, { exact: true }).waitFor();
  await page.getByTestId("issues-toggle").click();
  const panel = page.getByTestId("issues-panel");
  await panel.getByTestId("issues-title").click();
  const tracking = panel.getByTestId("issue-execution");
  await tracking.waitFor();
  const settled = () => panel.evaluate((root) => root.querySelector('[data-testid="issues-detail"]')?.getAttribute("aria-busy") === "false"
    && !root.querySelector('[data-testid="issues-detail-loading"]'));
  await until(settled, "the first tracking snapshot");
  assert.equal(await tracking.getByTestId("issue-execution-state").innerText(), execution.state_label);
  assert.equal(await tracking.getByTestId("issue-execution-attempts").innerText(), "2 / 3");
  assert.equal(await tracking.getByTestId("issue-verification-attempts").innerText(), "4회");
  assert.equal(await tracking.getByTestId("issue-publication-attempts").innerText(), "5회");
  assert.deepEqual(await tracking.getByTestId("issue-timeline-entry").evaluateAll((elements) => elements.map((entry) => Number(entry.dataset.sequence))), timeline.entries.map((entry) => entry.sequence));
  assert.deepEqual(await tracking.getByTestId("issue-timeline-entry").locator("p").allTextContents(), timeline.entries.map((entry) => entry.summary));
  assert.equal(await tracking.getByTestId("issue-timeline-entry").count(), 20);
  assert.equal(await tracking.getByTestId("issue-timeline-older").isVisible(), true);
  assert.equal(await tracking.getByTestId("issue-execution-timeline").getByRole("button").count(), 0);
  console.log("PASS server execution counts and the last twenty records preserve their separate values and order");

  const beforeReview = issueQueries().length;
  group = { id: "in_review", label: "리뷰 중" };
  execution = { ...execution, revision: 42, state: "in_review", state_label: "서버 리뷰 중", review_pull_request_url: prUrl };
  revision++;
  emit("issue_execution_progressed", 42);
  await until(async () => issueQueries().length > beforeReview && await settled()
    && await tracking.getByTestId("issue-execution-state").innerText() === execution.state_label, "recorded review progress");
  await tracking.getByTestId("issue-execution-pr").click();
  await panel.getByTestId("issues-detail-external").click();
  await until(async () => (await app.evaluate(() => globalThis.executionFixture.opened.length)) === 2, "both exact external URLs");
  assert.deepEqual(await app.evaluate(() => globalThis.executionFixture.opened), [prUrl, issueUrl]);
  console.log("PASS execution events refresh the detail and PR/issue links preserve the exact server URLs");

  group = { id: "done", label: "끝남" };
  execution = { ...execution, revision: 43, state: "done", state_label: "서버 통합 완료", terminal_class: "contract_recovery", recovery };
  feedback = { ...feedback, integration, reason: "feedback_unconfirmed" };
  revision++;
  emit("issue_integrated", 43);
  await until(async () => await settled() && await tracking.getByTestId("issue-execution-state").innerText() === execution.state_label, "integration with recovery");
  assert.equal(await tracking.getByTestId("issue-integration-state").innerText(), "통합 완료");
  assert.equal(await tracking.getByTestId("issue-external-state").innerText(), "외부 이슈 열림");
  assert.equal(await tracking.getByTestId("issue-feedback-unconfirmed").innerText(), "게시 결과 확인 필요");
  assert.equal(await tracking.getByTestId("issue-execution-recovery-summary").innerText(), recovery.summary);
  assert.equal(await tracking.getByTestId("issue-execution-next").innerText(), recovery.next_summary);
  const rightPanel = page.getByTestId("right-panel");
  await panel.getByTestId("issues-detail-back").scrollIntoViewIfNeeded();
  const compact = await rightPanel.boundingBox();
  assert.ok(compact);
  assert.equal(await tracking.evaluate((element) => element.scrollWidth <= element.clientWidth + 1), true);
  await capture("compact");
  await tracking.getByTestId("issue-execution-outcome").scrollIntoViewIfNeeded();
  await capture("outcome");
  await panel.getByTestId("issues-detail-back").scrollIntoViewIfNeeded();
  await page.getByTestId("issues-maximize").click();
  await until(async () => await rightPanel.getAttribute("data-maximized") === "true", "the maximized panel");
  const maximized = await rightPanel.boundingBox();
  assert.ok(maximized && maximized.width > compact.width);
  assert.equal(await tracking.evaluate((element) => element.scrollWidth <= element.clientWidth + 1), true);
  await capture("maximized");
  await tracking.getByTestId("issue-timeline-entry").last().scrollIntoViewIfNeeded();
  await capture("timeline");
  await page.getByTestId("issues-restore").click();
  await panel.getByTestId("issue-execution-refresh").scrollIntoViewIfNeeded();
  console.log("PASS Done, external Open, contract recovery and an unconfirmed publication remain visible together");

  const beforeRecheck = issueQueries().length;
  const writesBeforeRecheck = writes().length;
  feedback = { ...feedback, revision: 3, reason: null };
  await panel.getByTestId("issue-execution-refresh").click();
  await until(async () => issueQueries().length > beforeRecheck && await settled() && await tracking.getByTestId("issue-feedback-unconfirmed").count() === 0, "the query-only recheck");
  assert.equal(writes().length, writesBeforeRecheck);
  assert.equal(await tracking.getByTestId("issue-publication-attempts").innerText(), "5회");
  assert.deepEqual(issueQueries().at(-1).params, { query: { type: "issue", issue } });

  const beforeFeedback = issueQueries().length;
  feedback = { ...feedback, revision: 900, publication_attempts: 6 };
  emit("issue_feedback_posted", 900);
  await until(async () => issueQueries().length > beforeFeedback && await settled()
    && await tracking.getByTestId("issue-publication-attempts").innerText() === "6회", "feedback journal progress at the same catalog revision");
  feedback = { ...feedback, revision: 901, external_state: "closed" };
  emit("issue_closed", 901);
  await until(async () => await settled() && await tracking.getByTestId("issue-external-state").innerText() === "외부 이슈 닫힘", "the independent external closure");
  assert.equal(await tracking.getByTestId("issue-integration-state").innerText(), "통합 완료");
  assert.equal(await tracking.getByTestId("issue-execution-recovery-summary").innerText(), recovery.summary);
  console.log("PASS recheck only queries and feedback/closure events do not mix journal and catalog revisions");

  detailStatus = "unavailable";
  await panel.getByTestId("issue-execution-refresh").click();
  await until(async () => await settled() && await tracking.count() === 0, "an unavailable detail");
  assert.match(await panel.getByTestId("issues-detail").innerText(), /이슈 상세를 읽을 수 없습니다/);
  assert.equal(await panel.getByTestId("issue-execution-refresh").isEnabled(), true);
  assert.equal(await panel.getByTestId("issue-execution-empty").count(), 0);
  await capture("unavailable");
  detailStatus = "ready";
  execution = { status: "unavailable", reason: "read_failed" };
  timeline = { status: "unavailable" };
  await panel.getByTestId("issue-execution-refresh").click();
  await tracking.getByTestId("issue-execution-unavailable").waitFor();
  assert.equal(await tracking.getByTestId("issue-timeline-unavailable").isVisible(), true);
  assert.equal(await tracking.getByTestId("issue-execution-empty").count(), 0);
  assert.equal(await tracking.getByTestId("issue-integration-state").innerText(), "통합 완료");
  failNextDetail = true;
  await panel.getByTestId("issue-execution-refresh").click();
  await panel.getByTestId("issues-detail-error").waitFor();
  assert.equal(await panel.getByTestId("issue-execution-refresh").isEnabled(), true);
  console.log("PASS unavailable records and query errors remain explicit and keep the read-only recheck accessible");

  execution = {
    status: "ready", work_order_id: "work-order-execution", revision: 43, state: "done", state_label: "서버 통합 완료",
    execution_attempts: 2, execution_attempt_limit: 3, verification_attempts: 4,
    review_pull_request_url: prUrl, terminal_class: "contract_recovery", recovery,
  };
  let release;
  const replyAfter = new Promise((resolve) => { release = resolve; });
  const held = { replyAfter, release, requested: false };
  heldDetail = held;
  await panel.getByTestId("issue-execution-refresh").click();
  try {
    await until(() => held.requested, "the held detail query");
    await panel.getByTestId("issues-detail-loading").waitFor();
    assert.equal(await tracking.count(), 0);
    assert.equal(await panel.getByTestId("issue-execution-refresh").count(), 1);
    assert.equal(await panel.getByTestId("issue-execution-refresh").isDisabled(), true);
  } finally { held.release(); }
  await until(async () => await settled() && await tracking.count() === 1, "the resumed query");
  const beforeConversation = issueQueries().length;
  conversation = { kind: "thread", thread_id: targetThread };
  await panel.getByTestId("issue-open-conversation").click();
  await panel.waitFor({ state: "detached" });
  await page.getByText(targetBody, { exact: true }).waitFor();
  assert.ok(issueQueries().length > beforeConversation);
  assert.equal(requests.filter((request) => request.method === "thread/resume").at(-1).params.threadId, targetThread);
  assert.equal(await page.getByText(currentBody, { exact: true }).count(), 0);
  assert.equal(writes().length, 0);
  assert.equal(await app.evaluate(() => globalThis.executionFixture.dialogs.length), 0);
  assert.deepEqual(errors, []);
  console.log("PASS a fresh conversation query opens a different actual fake thread without starting a turn or taking over");
} catch (error) {
  if (page && !page.isClosed()) {
    const shown = await page.evaluate(() => document.querySelector('[data-testid="issues-panel"]')?.textContent).catch(() => null);
    console.error("execution tracking fixture failure", JSON.stringify({ revision, execution, feedback, shown }));
    await capture("failure").catch(() => undefined);
  }
  throw error;
} finally {
  if (app && appProcess?.exitCode === null) {
    const exited = new Promise((resolve) => appProcess.once("exit", resolve));
    for (const window of app.windows()) await window.close();
    let timer;
    const closed = await Promise.race([exited.then(() => true), new Promise((resolve) => { timer = setTimeout(() => resolve(false), 10_000); })]);
    clearTimeout(timer);
    if (!closed) { appProcess.kill(); throw new Error("hidden execution fixture did not close"); }
  }
  await gateway.close();
  await rm(userData, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 });
}
