// 실제 Electron IPC·utility confirmation을 가짜 Gateway에만 연결한다. 모델 턴은 시작하지 않는다.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { mkdtemp, mkdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { _electron as electron } from "playwright-core";
import { confirmationReply, confirmedRequest, createConfirmationFixture } from "../fixtures/confirmationBridge.mjs";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);
const workspace = "/work/example-app";
const issue = { provider: "github", server_id: "github.com", repository_id: "R_fixture", issue_id: "I_fixture", project_scope: { workspace_dir: workspace } };
const binding = { ...issue, id: "fixture-binding" };
delete binding.issue_id;
const requests = [];
let revision = 7;
let queriedAt = Date.parse("2026-11-21T09:00:00Z");
const decisionDueAt = "2026-11-22T09:00:00Z";
let level = "observe";
let effectiveOverride = null;
let cap = "propose";
let control = { tracked: true, held: false, taken_over: false, cancelled: false, authorization_epoch: 0 };
let nextDelivery = "queued";
let conversation = { kind: "draft", shared_copy: { title: "공유 작업 · 검색 응답 개선", body: "공유된 요구 사항을 검토하고 이어서 작업합니다." } };
let supersedeAnswer = false;
let heldDetailReply = null;
let heldInterventionReply = null;
let nextInterventionFailure = null;
const title = "검색 응답을 빠르게 하고 결과를 보존하기";
const question = "공유된 계획을 기준으로 이 작업을 시작할까요?";
const requestDecision = (id = "decision-start") => ({
  id, kind: "start_approval", subject: { kind: "issue", issue }, gate: "start", state: "open", revision: 1,
  basis: { policy_version: "runtime-policy-1", gate_revision: 1, authorization_epoch: control.authorization_epoch },
  question, options: [
    { id: "approve", label: "작업 시작 승인", effect: "현재 계획으로 작업을 시작합니다. 전달 단계에서는 별도의 허가를 확인합니다." },
    { id: "hold", label: "보류", effect: "작업을 시작하지 않고 이슈를 보류합니다." },
  ], recommended: "approve", on_timeout: "wait", due_at: decisionDueAt,
});
let openDecisions = [requestDecision()];
const fact = (request, state = request.state) => ({
  decision_id: request.id, kind: request.kind, subject: request.subject, gate: request.gate, basis: request.basis, state,
  ...(request.delivery_review ? { delivery_review: request.delivery_review } : {}),
});
let history = [{ decision: fact(requestDecision("prior-withheld"), "superseded"), resolution_reason: { code: "basis_changed", summary: "판정 기준이 바뀌어 이전 결정을 종료했습니다." } }];
const rank = (value) => ["off", "observe", "propose", "gated", "auto"].indexOf(value);
const autonomy = () => ({
  status: "ready", requested_level: level, effective_level: effectiveOverride ?? level, cap,
  revision, policy_version: "runtime-policy-" + revision, pending: false,
  choices: ["off", "observe", "propose", "gated", "auto"].map((choice) => ({
    level: choice, enabled: rank(choice) <= rank(cap), requires_confirmation: rank(choice) > rank(effectiveOverride ?? level),
    reason: rank(choice) > rank(cap) ? "서버 정책의 자율 수준 상한을 넘습니다." : null,
  })),
});
const state = (type = "issue_automation") => ({
  type, status: "ready", mode: "observe", epoch: 1, revision, binding, binding_id: binding.id, autonomy: autonomy(),
  collecting: false, consecutive_failures: 0, last_success_at_ms: Date.parse("2026-11-21T09:00:00Z"),
});
const item = () => ({
  issue, binding_id: binding.id, tracked: { state: "awaiting_decision", held: control.held, stale: false, unknown: false },
  current: { status: "observed", remote_state: "open", labels: { completeness: "complete", items: [] },
    assignees: { completeness: "complete", items: [] }, author: null, label_actors: { completeness: "complete", items: [] }, linked_pull_request: "absent",
    content: { kind: "shared", basis: { field_hash: { version: 1, sha256: "0".repeat(64) }, provider_updated_at: "2026-11-21T09:00:00Z" },
      shared_copy: { title, body: "## 목표\n검색 응답을 빠르게 하고 기존 결과를 보존합니다.\n\n## 완료 조건\n- 응답 시간 개선\n- 기존 검색 회귀 검증" }, policy_version: "disclosure-v1" } },
  presentation: { number: 2550, title, url: "https://github.com/sample/widgets/issues/2550",
    status: { code: control.held ? "held" : "awaiting_decision", label: control.held ? "보류됨" : "결정 대기" },
    reason: { code: "human_approval", summary: "작업을 시작하기 전에 사용자의 결정을 기다립니다." } },
});
const actions = () => ["hold_issue", "release_issue", "steer_issue", "take_over_issue", "retriage_issue", "cancel_issue_work", "track_issue", "untrack_issue"].map((command) => {
  const enabled = command === "release_issue" ? control.held && !control.taken_over && !control.cancelled
    : command === "hold_issue" ? !control.held && !control.cancelled
      : command === "track_issue" ? !control.tracked && !control.cancelled
        : command === "untrack_issue" ? control.tracked && !control.cancelled
          : command === "steer_issue" ? !control.taken_over && !control.cancelled
            : !control.cancelled;
  return { command, enabled, reason: enabled ? null : "현재 작업 상태에서 사용할 수 없습니다." };
});
const detail = () => ({ type: "issue", status: "ready", revision, as_of_unix_ms: queriedAt, item: item(), open_decisions: structuredClone(openDecisions),
  decision_history: structuredClone(history), control: { ...control }, actions: actions(), conversation: structuredClone(conversation) });
const commandRequests = (type) => requests.flatMap((entry) => {
  const request = entry.method === "akra/kernel/command_confirmed" ? entry.params.request : entry.method === "akra/kernel/command" ? entry.params : null;
  return request?.command?.type === type ? [{ ...entry, request }] : [];
});
const kernel = {
  kernel: "issue-interventions-fixture", queries: {},
  onRequest(method, params) {
    const confirmation = confirmationReply(method, params);
    if (confirmation) return confirmation;
    if (method === "akra/kernel/query") {
      if (params.query?.type === "issue_automation") return { result: state() };
      if (params.query?.type === "issues") return { result: { type: "issues", status: "ready", revision, items: [item()],
        groups: [{ id: "needs-decision", label: "결정이 필요한 이슈", item_indices: [0] }], next_after: null } };
      if (params.query?.type === "issue") {
        const reply = { result: detail() };
        if (!heldDetailReply) return reply;
        const held = heldDetailReply;
        heldDetailReply = null;
        held.requested = true;
        return { ...reply, replyAfter: held.replyAfter };
      }
      if (params.query?.type === "issue_automation_policy") return { result: { type: "issue_automation_policy", status: "ready", workspace, version: "file-policy", files: [],
        settings: [{ key: "select.labels_any", label: "선택 레이블", value: "akra:ready", resolution: "derived", resolution_summary: "워크플로에서 가져온 값입니다.", origins: [{ scope: "builtin", label: "기본 정책" }] }] } };
    }
    if (method !== "akra/kernel/command" && method !== "akra/kernel/command_confirmed") return undefined;
    const { command } = method === "akra/kernel/command_confirmed" ? confirmedRequest(params) : params;
    if (command.type === "set_issue_autonomy") {
      assert.ok(rank(command.level) <= rank(cap), "fake server must reject cap violations");
      if (rank(command.level) > rank(effectiveOverride ?? level)) assert.equal(method, "akra/kernel/command_confirmed");
      level = command.level; effectiveOverride = null; revision++;
      return { result: { processed: true, result: { type: "issue_autonomy_changed", command_id: command.command_id, changed: true, autonomy: autonomy() } } };
    }
    if (command.type === "answer_issue_decision") {
      assert.equal(method, "akra/kernel/command_confirmed");
      const current = openDecisions.find((value) => value.id === command.decision_id);
      assert.ok(current);
      if (current.kind === "agent_question" && command.option === "reply") {
        assert.equal(typeof command.note, "string");
        assert.ok(command.note.trim().length > 0, "an agent reply requires shared answer text");
      }
      revision++;
      const resolved = supersedeAnswer ? "superseded" : "answered";
      const decision = { ...fact(current, resolved), option: supersedeAnswer ? null : command.option, answer_by: supersedeAnswer ? null : "human" };
      const { arbiter_wait: _wait, ...definition } = current;
      history.unshift({ decision, request: { ...definition, state: resolved }, resolution_reason: { code: supersedeAnswer ? "basis_changed" : "answered",
        summary: supersedeAnswer ? "판정 기준이 바뀌어 이전 결정을 종료했습니다." : "사용자의 답변을 기록했습니다." } });
      openDecisions = openDecisions.filter((value) => value.id !== current.id);
      return { result: { processed: true, result: { type: "issue_decision_resolved", catalog_revision: revision, changed: true, decision, outcome: supersedeAnswer ? "superseded" : "answered" } } };
    }
    assert.deepEqual(command.issue, issue);
    assert.match(command.command_id, /^[0-9a-f-]{36}$/);
    if (nextInterventionFailure) {
      const failure = nextInterventionFailure;
      nextInterventionFailure = null;
      if (failure === "connection") return { error: { code: -32013, message: "fixture Gateway 응답을 확인하지 못했습니다." } };
      return { result: failure === "rejected"
        ? { processed: false, rejection: { reason: "issue_automation_off", message: "fixture 요청이 거절됐습니다.", next: [] } }
        : { processed: false, failure: "fixture 작업자 응답을 확인하지 못했습니다." } };
    }
    if (command.type === "cancel_issue_work") { assert.equal(command.confirm, true); control.cancelled = true; }
    if (command.type === "hold_issue") control.held = true;
    if (command.type === "release_issue") control.held = false;
    if (command.type === "take_over_issue") { control.taken_over = true; control.held = true; }
    if (command.type === "track_issue") control.tracked = true;
    if (command.type === "untrack_issue") control.tracked = false;
    if (command.type !== "steer_issue") control.authorization_epoch++;
    revision++;
    const reply = { result: { processed: true, result: { type: "issue_intervened", receipt: {
      issue, command_id: command.command_id, action: command.type, work_order_id: "work-order-fixture",
      authorization_epoch: control.authorization_epoch, changed: true, delivery: command.type === "steer_issue" ? nextDelivery : null,
    } } } };
    if (!heldInterventionReply) return reply;
    const held = heldInterventionReply;
    heldInterventionReply = null;
    held.requested = true;
    // 같은 socket의 후속 resync 읽기를 늦은 응답이 도착했다는 bounded barrier로 쓴다.
    return { ...reply, replyAfter: held.replyAfter, events: [{ type: "resync" }] };
  },
};
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  kernel, onRequest: (method, params) => requests.push({ method, params: structuredClone(params) }),
});
const userData = await mkdtemp(join(tmpdir(), "akra-issue-interventions-ui-"));
const fixture = await createConfirmationFixture(userData);
const env = { ...process.env, ...fixture.environment, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;
let app;
let appProcess;
let page;
const until = async (condition, description = "expected state") => {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    if (await condition()) return;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error("issue intervention fixture did not reach " + description);
};
const holdNextDetailReply = () => {
  assert.equal(heldDetailReply, null);
  let release;
  const replyAfter = new Promise((resolve) => { release = resolve; });
  const held = { replyAfter, release, requested: false };
  heldDetailReply = held;
  return held;
};
const holdNextInterventionReply = () => {
  assert.equal(heldInterventionReply, null);
  let release;
  const replyAfter = new Promise((resolve) => { release = resolve; });
  const held = { replyAfter, release, requested: false };
  heldInterventionReply = held;
  return held;
};
const capture = async (suffix) => {
  if (!process.env.AKRA_QA_ISSUE_INTERVENTIONS_SCREENSHOT) return;
  const path = process.env.AKRA_QA_ISSUE_INTERVENTIONS_SCREENSHOT.replace(/\.png$/i, "") + "-" + suffix + ".png";
  await mkdir(dirname(path), { recursive: true });
  const window = await app.browserWindow(page);
  const original = await window.evaluate((owner) => ({ visible: owner.isVisible(), minimized: owner.isMinimized() }));
  // 가상 화면임을 명시한 검증에서만 창을 잠시 표시해 최신 프레임을 캡처한다.
  const isolatedDisplay = process.env.AKRA_QA_ISOLATED_DISPLAY;
  const reveal = !original.visible && isolatedDisplay !== undefined;
  if (reveal) {
    assert.equal(process.platform, "linux");
    assert.equal(process.env.DISPLAY, isolatedDisplay);
    assert.match(isolatedDisplay, /^:[1-9][0-9]*(?:[.][0-9]+)?$/);
  }
  try {
    if (reveal) await window.evaluate((owner) => owner.showInactive());
    await page.screenshot({ path, animations: "disabled", timeout: 15_000 });
  } finally {
    if (reveal) await window.evaluate((owner, state) => {
      if (owner.isDestroyed()) return;
      if (state.minimized) owner.minimize();
      owner.hide();
    }, original);
    await window.dispose();
  }
};
try {
  app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env, cwd: fixture.cwd });
  appProcess = app.process();
  await app.evaluate(({ ipcMain, dialog }) => {
    globalThis.issueInterventionFixture = { dialogs: [], nextAnswer: 1, resolve: null, focused: false, focusChecks: 0 };
    dialog.showMessageBox = (_owner, options) => {
      const fixture = globalThis.issueInterventionFixture;
      fixture.dialogs.push({ message: options.message, detail: options.detail, defaultId: options.defaultId, cancelId: options.cancelId });
      if (fixture.nextAnswer !== null) return Promise.resolve({ response: fixture.nextAnswer, checkboxChecked: false });
      return new Promise((resolve) => { fixture.resolve = (response) => resolve({ response, checkboxChecked: false }); });
    };
    ipcMain.removeHandler("akra:app-focused");
    ipcMain.handle("akra:app-focused", () => {
      globalThis.issueInterventionFixture.focusChecks++;
      return globalThis.issueInterventionFixture.focused;
    });
  });
  page = await app.firstWindow();
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  // firstWindow() can resolve before Electron has installed the isolated preload bridge.
  await page.waitForFunction(() => typeof window.akra?.settings?.set === "function", undefined, { timeout: 30_000 });
  // 숨긴 fixture는 실제 운영체제 알림을 만들지 않는다. 알림 이동은 아래에서 명시적으로 시험한다.
  await page.evaluate(async () => {
    window.issueNotifications = [];
    class FixtureNotification {
      static permission = "granted";
      constructor(title, options) { this.title = title; this.options = options; window.issueNotifications.push(this); }
      close() {}
    }
    Object.defineProperty(window, "Notification", { value: FixtureNotification, configurable: true });
    await window.akra.settings.set({ notifications: { mode: "always", sound: false } });
  });

  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("issues-toggle").click();
  const panel = page.getByTestId("issues-panel");
  const select = panel.getByTestId("issues-autonomy-level");
  const effectiveLevel = panel.getByTestId("issues-autonomy-effective").locator("strong");
  await until(async () => await select.isEnabled());
  assert.equal(await select.inputValue(), "observe");
  await select.selectOption("propose");
  await until(async () => (await app.evaluate(() => globalThis.issueInterventionFixture.dialogs.length)) === 1 && await select.isEnabled());
  assert.equal(commandRequests("set_issue_autonomy").length, 0);
  assert.equal(await select.inputValue(), "observe");
  await app.evaluate(() => { globalThis.issueInterventionFixture.nextAnswer = 0; });
  await select.selectOption("propose");
  await until(async () => level === "propose" && await select.isEnabled());
  assert.equal(commandRequests("set_issue_autonomy").length, 1);
  assert.equal(commandRequests("set_issue_autonomy")[0].method, "akra/kernel/command_confirmed");
  cap = "gated"; revision++;
  gateway.sendKernelEvent(state("issues"));
  await until(async () => !(await select.locator('option[value="gated"]').evaluate((option) => option.disabled)), "the enabled Gated choice");
  await app.evaluate(() => { globalThis.issueInterventionFixture.nextAnswer = 1; });
  const beforeGatedDialogs = await app.evaluate(() => globalThis.issueInterventionFixture.dialogs.length);
  await select.selectOption("gated");
  await until(async () => (await app.evaluate(() => globalThis.issueInterventionFixture.dialogs.length)) > beforeGatedDialogs && await select.isEnabled());
  assert.equal(level, "propose");
  assert.equal(commandRequests("set_issue_autonomy").length, 1);
  await app.evaluate(() => { globalThis.issueInterventionFixture.nextAnswer = 0; });
  await select.selectOption("gated");
  await until(async () => level === "gated" && await select.inputValue() === "gated" && await select.isEnabled());
  assert.equal(commandRequests("set_issue_autonomy").length, 2);
  assert.equal(commandRequests("set_issue_autonomy")[1].method, "akra/kernel/command_confirmed");
  cap = "auto"; revision++;
  gateway.sendKernelEvent(state("issues"));
  await until(async () => !(await select.locator('option[value="auto"]').evaluate((option) => option.disabled)), "the enabled Auto choice");
  await app.evaluate(() => { globalThis.issueInterventionFixture.nextAnswer = 1; });
  const beforeAutoDialogs = await app.evaluate(() => globalThis.issueInterventionFixture.dialogs.length);
  await select.selectOption("auto");
  await until(async () => (await app.evaluate(() => globalThis.issueInterventionFixture.dialogs.length)) > beforeAutoDialogs && await select.isEnabled());
  assert.equal(level, "gated");
  assert.equal(await select.inputValue(), "gated");
  assert.equal(commandRequests("set_issue_autonomy").length, 2);
  await app.evaluate(() => { globalThis.issueInterventionFixture.nextAnswer = 0; });
  await select.selectOption("auto");
  await until(async () => level === "auto" && await select.inputValue() === "auto" && await select.isEnabled()
    && await effectiveLevel.innerText() === "자동", "the confirmed Auto effective level");
  assert.equal(commandRequests("set_issue_autonomy").length, 3);
  assert.equal(commandRequests("set_issue_autonomy")[2].method, "akra/kernel/command_confirmed");
  assert.equal(commandRequests("set_issue_autonomy")[2].request.command.level, "auto");
  assert.match(await panel.getByTestId("issues-autonomy-auto-help").innerText(), /공개 저장소.*필수 검증.*사람 승인.*이번 실행.*PR에 사람 승인/s);
  cap = "propose"; effectiveOverride = "propose"; revision++;
  gateway.sendKernelEvent(state("issues"));
  await until(async () => await effectiveLevel.innerText() === "제안", "the server-restricted Auto effective level");
  assert.equal(await select.inputValue(), "auto", "a server restriction does not rewrite the requested choice");
  assert.equal(commandRequests("set_issue_autonomy").length, 3, "an effective-level refresh does not submit another command");
  cap = "observe"; level = "observe"; effectiveOverride = null; revision++;
  gateway.sendKernelEvent(state("issues"));
  await until(async () => await select.inputValue() === "observe" && await select.locator('option[value="propose"]').evaluate((option) => option.disabled), "the pushed lower autonomy cap");
  assert.equal(await select.locator('option[value="gated"]').evaluate((option) => option.disabled), true);
  assert.equal(await select.locator('option[value="auto"]').evaluate((option) => option.disabled), true);
  assert.deepEqual(await panel.getByTestId("issues-autonomy-reason").allTextContents(), [
    "제안: 서버 정책의 자율 수준 상한을 넘습니다.",
    "단계별 승인: 서버 정책의 자율 수준 상한을 넘습니다.",
    "자동: 서버 정책의 자율 수준 상한을 넘습니다.",
  ]);
  cap = "auto"; level = null; effectiveOverride = "propose"; revision++;
  gateway.sendKernelEvent(state("issues"));
  await until(async () => await select.inputValue() === "propose" && await effectiveLevel.innerText() === "제안",
    "the proposal ceiling without an explicit runtime choice");
  assert.equal(commandRequests("set_issue_autonomy").length, 3, "file policy alone does not submit or grant an Auto choice");
  await capture("autonomy");
  cap = "propose"; level = "propose"; effectiveOverride = null; revision++;
  gateway.sendKernelEvent(state("issues"));
  await until(async () => await select.inputValue() === "propose");
  console.log("PASS native Gated/Auto consent, cancellation, cap reasons, server-restricted Auto and the unconfirmed policy ceiling");

  await panel.getByTestId("issues-title").click();
  const currentCard = () => panel.locator('[data-testid="issue-decision-card"][data-state="open"]').first();
  await currentCard().waitFor();
  assert.equal(await currentCard().getByTestId("issue-delivery-review").count(), 0, "Start has no delivery review");
  assert.match(await currentCard().getByTestId("issue-decision-remaining").innerText(), /1일 남음.*조회 기준/s);
  assert.match(await currentCard().getByTestId("issue-decision-timeout").innerText(), /기한 후에도 사람의 답변을 기다립니다/);

  // 최신 이슈 상세의 제품 버튼이 전체 대상 좌표로 전용 창을 연다. 자동화 프로필은 native 복원을 차단한다.
  const originalWindowOpened = app.waitForEvent("window");
  await panel.getByTestId("issues-detail-original").click();
  const originalWindow = await originalWindowOpened;
  await originalWindow.locator('#root[data-phase="held"]').waitFor({ timeout: 10_000 });
  assert.match(await originalWindow.getByTestId("vault-restore-detail").innerText(), /자동화/);
  assert.equal(await originalWindow.getByTestId("vault-restored-body").count(), 0);
  await originalWindow.close();
  console.log("PASS current issue original button opens a separate restore window under the profile hold");
  assert.equal(await currentCard().getByTestId("issue-decision-question").innerText(), question);
  assert.match(await currentCard().innerText(), /권고/);
  assert.equal(await currentCard().getByTestId("issue-decision-answer").isDisabled(), true);
  await capture("compact");
  await page.getByTestId("issues-maximize").click();
  await capture("expanded");
  const draftNote = "백그라운드 조회에도 보존할 공유 메모";
  const draftMessage = "아직 보내지 않은 작업자 지시";
  await currentCard().locator('input[value="approve"]').check();
  await currentCard().getByTestId("issue-decision-note").fill(draftNote);
  await panel.getByTestId("issue-action-steer_issue").click();
  await panel.getByTestId("issue-steer-message").fill(draftMessage);
  const draftCommandCounts = () => ["answer_issue_decision", "steer_issue", "cancel_issue_work"].map((name) => commandRequests(name).length);
  const beforeDraftCommands = draftCommandCounts();
  const beforeDraftDialogs = await app.evaluate(() => globalThis.issueInterventionFixture.dialogs.length);
  const refreshDrafts = async (description) => {
    const held = holdNextDetailReply();
    revision++;
    gateway.sendKernelEvent(state("issues"));
    try {
      await until(() => held.requested, description + " detail request");
      await panel.getByTestId("issues-detail-loading").waitFor();
      assert.equal(await currentCard().count(), 0, "stale server decisions stay hidden while the reply is held");
      assert.equal(await panel.getByTestId("issues-shared-copy").count(), 0);
      assert.equal(await panel.getByTestId("issues-detail-original").count(), 0, "a stale detail cannot open a native restore target");
      assert.equal(await panel.getByTestId("issue-steer-message").count(), 0);
      assert.equal(await panel.getByTestId("issue-cancel-confirm").count(), 0);
      assert.deepEqual(draftCommandCounts(), beforeDraftCommands);
    } finally {
      held.release();
    }
    await currentCard().waitFor();
    await until(async () => (await panel.getByTestId("issues-detail-loading").count()) === 0, description + " detail response");
  };
  await refreshDrafts("same basis");
  assert.equal(await currentCard().locator('input[value="approve"]').isChecked(), true);
  assert.equal(await currentCard().getByTestId("issue-decision-note").inputValue(), draftNote);
  assert.equal(await panel.getByTestId("issue-steer-message").inputValue(), draftMessage);
  assert.equal(await panel.getByTestId("issue-steer-send").isEnabled(), true);

  // 서버 조회 시각과 재알림만 바뀌어도 작성 중인 답과 native confirmation 대상은 유지한다.
  queriedAt = Date.parse(decisionDueAt) - 2 * 60 * 60 * 1000;
  revision++;
  const reminder = { type: "recorded", sequence: 1, recorded_at: new Date(queriedAt).toISOString(),
    event: { kind: "issue_decision_reminded", event_id: "issue-decision-reminder:fixture", decision: fact(openDecisions[0]),
      due_at: decisionDueAt, reminded_at: new Date(queriedAt).toISOString(), revision } };
  const reminderRead = holdNextDetailReply();
  const notificationsBeforeReminder = await page.evaluate(() => window.issueNotifications.length);
  gateway.sendKernelEvent(reminder);
  try {
    await until(() => reminderRead.requested, "reminder detail request");
    await panel.getByTestId("issues-detail-loading").waitFor();
    assert.deepEqual(draftCommandCounts(), beforeDraftCommands);
  } finally { reminderRead.release(); }
  await currentCard().waitFor();
  await until(async () => (await panel.getByTestId("issues-detail-loading").count()) === 0, "reminder detail response");
  await until(async () => (await page.evaluate(() => window.issueNotifications.length)) === notificationsBeforeReminder + 1, "one reminder notification");
  assert.match(await currentCard().getByTestId("issue-decision-remaining").innerText(), /2시간 남음.*조회 기준/s);
  assert.equal(await currentCard().locator('input[value="approve"]').isChecked(), true);
  assert.equal(await currentCard().getByTestId("issue-decision-note").inputValue(), draftNote);
  assert.match(await page.evaluate(() => window.issueNotifications.at(-1).options.body), /응답 기한이 다가옵니다/);
  gateway.sendKernelEvent(reminder);
  queriedAt = Date.parse(decisionDueAt) + 1;
  // The later refresh is also a socket-order barrier after the duplicate reminder.
  await refreshDrafts("overdue wait after duplicate reminder");
  assert.match(await currentCard().getByTestId("issue-decision-remaining").innerText(), /기한 지남.*조회 기준/s);
  assert.equal(await currentCard().locator('input[value="approve"]').isChecked(), true);
  assert.equal(await currentCard().getByTestId("issue-decision-note").inputValue(), draftNote);
  assert.equal(await page.evaluate(() => window.issueNotifications.length), notificationsBeforeReminder + 1);
  assert.equal(await app.evaluate(() => globalThis.issueInterventionFixture.dialogs.length), beforeDraftDialogs);
  assert.deepEqual(draftCommandCounts(), beforeDraftCommands);
  console.log("PASS server-time deadline, one durable reminder and draft preservation through overdue Wait");

  await panel.getByTestId("issue-action-cancel_issue_work").click();
  await panel.getByTestId("issue-cancel-confirm").waitFor();
  await refreshDrafts("same basis cancel form");
  assert.equal(await panel.getByTestId("issue-cancel-confirm").isVisible(), true);
  assert.equal(await currentCard().getByTestId("issue-decision-note").inputValue(), draftNote);

  const originalDecisionId = openDecisions[0].id;
  const originalDecisionRevision = openDecisions[0].revision;
  control = { ...control, authorization_epoch: control.authorization_epoch + 1 };
  openDecisions = openDecisions.map((request) => ({
    ...request, basis: { ...request.basis, gate_revision: request.basis.gate_revision + 1, authorization_epoch: control.authorization_epoch },
  }));
  await refreshDrafts("changed basis");
  assert.equal(openDecisions[0].id, originalDecisionId);
  assert.equal(openDecisions[0].revision, originalDecisionRevision, "basis identity must not rely on the decision revision alone");
  assert.equal(await currentCard().locator('input[value="approve"]').isChecked(), false);
  assert.equal(await currentCard().getByTestId("issue-decision-note").inputValue(), "");
  assert.equal(await currentCard().getByTestId("issue-decision-answer").isDisabled(), true);
  assert.equal(await panel.getByTestId("issue-cancel-confirm").count(), 0);
  assert.equal(await panel.getByTestId("issue-steer-message").count(), 0);
  await panel.getByTestId("issue-action-steer_issue").click();
  assert.equal(await panel.getByTestId("issue-steer-message").inputValue(), "");
  assert.equal(await panel.getByTestId("issue-steer-send").isDisabled(), true);
  await panel.getByTestId("issue-action-steer_issue").click();
  assert.deepEqual(draftCommandCounts(), beforeDraftCommands);
  assert.equal(await app.evaluate(() => globalThis.issueInterventionFixture.dialogs.length), beforeDraftDialogs);
  console.log("PASS same-basis refresh preserves decision and intervention drafts; changed basis cannot submit old forms");

  await currentCard().locator('input[value="approve"]').check();
  assert.equal(await currentCard().getByTestId("issue-decision-answer").isEnabled(), true, "a regular approval keeps the shared note optional");
  assert.equal(await currentCard().getByTestId("issue-decision-note").getAttribute("aria-required"), "false");
  await currentCard().getByTestId("issue-decision-note").fill("공유 가능한 검증 메모입니다.");
  await app.evaluate(() => { globalThis.issueInterventionFixture.nextAnswer = 1; });
  await currentCard().getByTestId("issue-decision-answer").click();
  await until(async () => await currentCard().getByTestId("issue-decision-answer").isEnabled());
  assert.equal(commandRequests("answer_issue_decision").length, 0);
  assert.equal(await currentCard().locator('input[value="approve"]').isChecked(), true);
  assert.equal(await currentCard().getByTestId("issue-decision-note").inputValue(), "공유 가능한 검증 메모입니다.");
  await app.evaluate(() => { globalThis.issueInterventionFixture.nextAnswer = 0; });
  await currentCard().getByTestId("issue-decision-answer").click();
  // 카드가 재조회 중 잠시 사라진 순간이 아니라 이전 명령의 완료 표시를 기다린다.
  await until(async () => commandRequests("answer_issue_decision").length === 1 && await panel.evaluate((root) => {
    const detail = root.querySelector('[data-testid="issues-detail"]');
    const holdAction = root.querySelector('[data-testid="issue-action-hold_issue"]');
    return detail?.getAttribute("aria-busy") === "false"
      && !root.querySelector('[data-testid="issues-detail-loading"]')
      && root.querySelector('[data-testid="issues-command-notice"]')?.textContent === "요청을 처리했습니다."
      && holdAction !== null && !holdAction.matches(":disabled")
      && !root.querySelector('[data-testid="issue-decision-card"][data-state="open"]');
  }), "the first decision command and its detail refresh to finish");
  assert.equal(commandRequests("answer_issue_decision")[0].request.command.note, "공유 가능한 검증 메모입니다.");
  assert.equal(await panel.locator('[data-state="answered"] input').first().isDisabled(), true);
  openDecisions = [requestDecision("decision-stale")]; revision++;
  gateway.sendKernelEvent({ type: "issue_decision", decision: fact(openDecisions[0]), revision });
  const staleCard = panel.locator('[data-testid="issue-decision-card"][data-decision-id="decision-stale"][data-state="open"]');
  // 새 결정의 조회 상태와 입력 가능 여부를 같은 DOM snapshot에서 확인한다.
  const staleDecisionReady = (selected = false) => panel.evaluate((root, selected) => {
    const detail = root.querySelector('[data-testid="issues-detail"]');
    const card = root.querySelector('[data-testid="issue-decision-card"][data-decision-id="decision-stale"][data-state="open"]');
    const option = card?.querySelector('input[value="approve"]');
    const answer = card?.querySelector('[data-testid="issue-decision-answer"]');
    return detail?.getAttribute("aria-busy") === "false"
      && !root.querySelector('[data-testid="issues-detail-loading"]')
      && option !== undefined && option !== null && !option.matches(":disabled")
      && answer !== undefined && answer !== null
      && (!selected || (option.checked && !answer.matches(":disabled")));
  }, selected);
  await until(() => staleDecisionReady(), "the new decision-stale snapshot to be ready");
  await staleCard.locator('input[value="approve"]').check();
  const notificationDraft = "같은 이슈 알림에서도 보존할 공유 메모";
  await staleCard.getByTestId("issue-decision-note").fill(notificationDraft);
  const staleTag = "akra:issue-decision:issue-interventions-fixture:decision-stale";
  await until(() => page.evaluate((tag) => window.issueNotifications.some((notice) => notice.options.tag === tag), staleTag), "the same-issue notification");
  const beforeNotificationReads = requests.filter((entry) => entry.params.query?.type === "issue").length;
  await page.evaluate((tag) => window.issueNotifications.find((notice) => notice.options.tag === tag).onclick(), staleTag);
  await until(async () => requests.filter((entry) => entry.params.query?.type === "issue").length > beforeNotificationReads
    && await staleDecisionReady(), "a fresh detail after opening the same-issue notification");
  assert.equal(await staleCard.locator('input[value="approve"]').isChecked(), true, "opening the same issue must preserve its selected answer");
  assert.equal(await staleCard.getByTestId("issue-decision-note").inputValue(), notificationDraft);

  await app.evaluate(() => { globalThis.issueInterventionFixture.nextAnswer = null; });
  const dialogCount = await app.evaluate(() => globalThis.issueInterventionFixture.dialogs.length);
  await until(() => staleDecisionReady(true), "decision-stale to retain its choice and enable its answer");
  await staleCard.getByTestId("issue-decision-answer").click();
  await until(async () => (await app.evaluate(() => globalThis.issueInterventionFixture.dialogs.length)) > dialogCount);
  supersedeAnswer = true;
  await app.evaluate(() => globalThis.issueInterventionFixture.resolve(0));
  await until(async () => (await panel.locator('[data-testid="issue-decision-card"][data-state="open"]').count()) === 0 && commandRequests("answer_issue_decision").length === 2);
  assert.match(await panel.getByTestId("issues-command-notice").innerText(), /이전 결정에 답하지 않았습니다/);
  const superseded = panel.locator('[data-testid="issue-decision-card"][data-state="superseded"]');
  assert.equal(await superseded.locator("input").first().isDisabled(), true);
  assert.match(await superseded.innerText(), /판정 기준이 바뀌어/);
  console.log("PASS issue-only cards, native cancellation, one answer and superseded disabled history");

  const waitingQuestion = {
    ...requestDecision("decision-agent-question"),
    kind: "agent_question", gate: "spec",
    basis: { ...requestDecision().basis, work_order_id: "work-order-fixture" },
    question: "원래 대화를 확인한 뒤 작업자 질문에 답변해 주세요.",
    options: [{ id: "reply", label: "답변 보내기", effect: "공유한 답변을 같은 작업 대화에 전달합니다." }],
    recommended: "reply",
  };
  supersedeAnswer = false;
  openDecisions = [waitingQuestion]; revision++;
  gateway.sendKernelEvent({ type: "issue_decision", decision: fact(waitingQuestion), revision });
  await currentCard().waitFor();
  await currentCard().locator('input[value="reply"]').check();
  const questionNote = currentCard().getByTestId("issue-decision-note");
  const questionAnswer = currentCard().getByTestId("issue-decision-answer");
  const answersBeforeQuestion = commandRequests("answer_issue_decision").length;
  const dialogsBeforeQuestion = await app.evaluate(() => globalThis.issueInterventionFixture.dialogs.length);
  assert.match(await currentCard().locator(".issues-input-label").innerText(), /공유할 답변\s*\(필수\)/);
  assert.equal(await questionNote.getAttribute("aria-required"), "true");
  assert.equal(await questionAnswer.isDisabled(), true);
  await questionNote.fill(" \t\n ");
  assert.equal(await questionAnswer.isDisabled(), true);
  assert.equal(commandRequests("answer_issue_decision").length, answersBeforeQuestion);
  assert.equal(await app.evaluate(() => globalThis.issueInterventionFixture.dialogs.length), dialogsBeforeQuestion);
  const answerText = "공유 답변: 기존 검색 결과의 정렬 순서를 유지해 주세요.";
  await questionNote.fill(answerText);
  await until(async () => await questionAnswer.isEnabled());
  await capture("agent-question");
  await app.evaluate(() => { globalThis.issueInterventionFixture.nextAnswer = 0; });
  await questionAnswer.click();
  await until(async () => commandRequests("answer_issue_decision").length === answersBeforeQuestion + 1
    && (await panel.locator('[data-testid="issue-decision-card"][data-state="open"]').count()) === 0);
  const sentAnswer = commandRequests("answer_issue_decision").at(-1);
  assert.equal(sentAnswer.method, "akra/kernel/command_confirmed");
  assert.equal(sentAnswer.request.command.option, "reply");
  assert.equal(sentAnswer.request.command.note, answerText);
  console.log("PASS agent-question reply requires nonblank shared text and keeps ordinary notes optional");

  const action = (name) => panel.getByTestId("issue-action-" + name);
  await action("hold_issue").click();
  await until(async () => control.held && await action("release_issue").isEnabled());
  assert.match(await panel.getByTestId("issue-control-state").innerText(), /보류/);
  await action("release_issue").click();
  await until(async () => !control.held && await action("hold_issue").isEnabled());
  for (const delivery of ["queued", "sent", "unknown"]) {
    nextDelivery = delivery;
    await action("steer_issue").click();
    await panel.getByTestId("issue-steer-message").fill("시험 추가 지시 " + delivery);
    await panel.getByTestId("issue-steer-send").click();
    await until(async () => commandRequests("steer_issue").some((entry) => entry.request.command.message.endsWith(delivery)) && await action("steer_issue").isEnabled());
    if (delivery === "unknown") {
      assert.equal(await panel.getByTestId("issue-steer-message").inputValue(), "시험 추가 지시 unknown");
      assert.equal(await panel.getByTestId("issue-steer-send").isEnabled(), true);
    } else {
      await panel.getByTestId("issue-steer-message").waitFor({ state: "detached" });
      await action("steer_issue").click();
      assert.equal(await panel.getByTestId("issue-steer-message").inputValue(), "");
      await action("steer_issue").click();
    }
  }
  assert.match(await panel.getByTestId("issues-command-notice").innerText(), /자동으로 다시 보내지 않습니다/);
  const steerCommands = commandRequests("steer_issue");
  assert.equal(steerCommands.length, 3);
  assert.equal(new Set(steerCommands.map((entry) => entry.request.command.command_id)).size, 3);
  const attachBeforeReconnect = requests.filter((entry) => entry.method === "akra/kernel/attach").length;
  gateway.disconnectClients();
  await until(async () => requests.filter((entry) => entry.method === "akra/kernel/attach").length > attachBeforeReconnect
    && await page.getByTestId("connection-phase").innerText() === "연결됨"
    && await panel.getByTestId("issues-title").count() === 1);
  assert.equal(commandRequests("steer_issue").length, 3);
  assert.match(await panel.getByTestId("issues-command-notice").innerText(), /자동으로 다시 보내지 않습니다/);
  await panel.getByTestId("issues-title").click();
  await until(async () => await action("untrack_issue").isEnabled());
  await action("untrack_issue").click();
  await until(async () => !control.tracked && await action("track_issue").isEnabled());
  await action("track_issue").click();
  await until(async () => control.tracked && await action("untrack_issue").isEnabled());
  await action("retriage_issue").click();
  await until(async () => commandRequests("retriage_issue").length === 1 && await action("retriage_issue").isEnabled());
  console.log("PASS durable intervention commands, all steer receipts and no replay after reconnect");

  const conversationQueries = requests.filter((entry) => entry.params.query?.type === "issue").length;
  await panel.getByTestId("issue-open-conversation").click();
  await page.getByTestId("home").waitFor();
  assert.equal(await page.getByTestId("composer-input").inputValue(), conversation.shared_copy.title + "\n\n" + conversation.shared_copy.body);
  assert.ok(requests.filter((entry) => entry.params.query?.type === "issue").length > conversationQueries);
  assert.equal(commandRequests("take_over_issue").length, 0);
  await page.getByTestId("issues-toggle").click();
  await panel.getByTestId("issues-title").click();
  const resumedThread = requests.find((entry) => entry.method === "thread/resume")?.params.threadId;
  assert.ok(resumedThread);
  conversation = { kind: "thread", thread_id: resumedThread };
  await panel.getByTestId("issue-open-conversation").click();
  await panel.waitFor({ state: "detached" });
  assert.equal(commandRequests("take_over_issue").length, 0);
  await page.getByTestId("issues-toggle").click();
  await panel.getByTestId("issues-title").click();
  await action("take_over_issue").click();
  await until(async () => control.taken_over && await action("cancel_issue_work").isEnabled());
  assert.equal(commandRequests("take_over_issue").length, 1);
  await action("cancel_issue_work").click();
  assert.equal(commandRequests("cancel_issue_work").length, 0);
  await panel.getByTestId("issue-cancel-confirm").click();
  await until(async () => control.cancelled && await action("cancel_issue_work").isDisabled());
  assert.equal(commandRequests("cancel_issue_work")[0].request.command.confirm, true);
  assert.equal(await panel.getByTestId("issue-cancel-confirm").count(), 0);
  console.log("PASS fresh conversation navigation creates only a draft/read; takeover and cancel remain explicit");

  control = { ...control, held: false, taken_over: false, cancelled: false, authorization_epoch: control.authorization_epoch + 1 };
  revision++;
  gateway.sendKernelEvent(state("issues"));
  await until(async () => await action("steer_issue").isEnabled());
  await action("steer_issue").click();
  const retainedMessage = "응답 실패 뒤에도 보존할 작업자 지시";
  await panel.getByTestId("issue-steer-message").fill(retainedMessage);
  for (const failure of ["rejected", "failure", "connection"]) {
    const before = commandRequests("steer_issue").length;
    nextInterventionFailure = failure;
    await panel.getByTestId("issue-steer-send").click();
    await until(async () => commandRequests("steer_issue").length === before + 1
      && await panel.getByTestId("issue-steer-send").isEnabled()
      && await panel.getByTestId("issues-error").count() === 1, failure + " preserves the steer form");
    assert.equal(await panel.getByTestId("issue-steer-message").inputValue(), retainedMessage);
    assert.equal(commandRequests("steer_issue").length, before + 1);
  }
  await action("cancel_issue_work").click();
  const beforeRejectedCancel = commandRequests("cancel_issue_work").length;
  nextInterventionFailure = "rejected";
  await panel.getByTestId("issue-cancel-confirm").click();
  await until(async () => commandRequests("cancel_issue_work").length === beforeRejectedCancel + 1
    && await panel.getByTestId("issue-cancel-confirm").isEnabled(), "a rejected cancel keeps its confirmation form");
  assert.equal(await panel.getByTestId("issue-cancel-confirm").isVisible(), true);
  await panel.getByRole("button", { name: "돌아가기", exact: true }).click();
  assert.equal(commandRequests("cancel_issue_work").length, beforeRejectedCancel + 1);
  await action("steer_issue").click();
  assert.equal(await panel.getByTestId("issue-steer-message").inputValue(), retainedMessage);
  console.log("PASS kernel rejection, failure and connection errors preserve drafts; rejected cancel preserves its form");

  for (const changedBasis of [false, true]) {
    const held = holdNextInterventionReply();
    nextDelivery = "queued";
    const before = commandRequests("steer_issue").length;
    await panel.getByTestId("issue-steer-message").fill("이전 선택에서 보낸 지시");
    await panel.getByTestId("issue-steer-send").click();
    await until(() => held.requested, "the delayed intervention receipt");
    assert.equal(await panel.getByTestId("issue-steer-message").inputValue(), "이전 선택에서 보낸 지시");
    try {
      await page.getByTestId("issues-toggle").click();
      await panel.waitFor({ state: "detached" });
      if (changedBasis) control = { ...control, authorization_epoch: control.authorization_epoch + 1 };
      revision++;
      await page.getByTestId("issues-toggle").click();
      await panel.getByTestId("issues-title").click();
      await until(async () => await action("steer_issue").isEnabled());
      await action("steer_issue").click();
      const freshMessage = changedBasis ? "새 기준에서 작성한 지시" : "같은 기준에서 새로 작성한 지시";
      await panel.getByTestId("issue-steer-message").fill(freshMessage);
      const detailReads = requests.filter((entry) => entry.params.query?.type === "issue").length;
      held.release();
      await until(async () => requests.filter((entry) => entry.params.query?.type === "issue").length > detailReads
        && await panel.getByTestId("issue-steer-send").isEnabled(), "the resync after a late receipt");
      assert.equal(await panel.getByTestId("issue-steer-message").inputValue(), freshMessage);
      assert.equal(commandRequests("steer_issue").length, before + 1);
    } finally {
      held.release();
    }
  }
  console.log("PASS pending submission preserves its draft and a late success cannot erase a newer draft or basis");


  await page.getByTestId("issues-toggle").click();
  await panel.waitFor({ state: "detached" });
  await page.evaluate(() => { window.issueNotifications = []; });
  await app.evaluate(() => { globalThis.issueInterventionFixture.focused = true; });
  const checks = await app.evaluate(() => globalThis.issueInterventionFixture.focusChecks);
  revision++;
  gateway.sendKernelEvent({ type: "issue_decision", decision: fact(requestDecision("foreground-decision")), revision });
  await until(async () => (await app.evaluate(() => globalThis.issueInterventionFixture.focusChecks)) > checks);
  assert.equal(await page.evaluate(() => window.issueNotifications.length), 0);
  await app.evaluate(() => { globalThis.issueInterventionFixture.focused = false; });
  control = { tracked: true, held: false, taken_over: false, cancelled: false, authorization_epoch: control.authorization_epoch + 1 };
  openDecisions = [requestDecision("background-decision")]; revision++;
  const backgroundFact = fact(openDecisions[0]);
  const beforeBackgroundReads = requests.filter((entry) => entry.method === "akra/kernel/query").length;
  gateway.sendKernelEvent({ type: "issue_decision", decision: backgroundFact, revision });
  gateway.sendKernelEvent({ type: "recorded", sequence: 100, recorded_at: "2026-11-21T09:01:00Z", event: { kind: "issue_decision_requested", decision: backgroundFact, revision } });
  await until(async () => await page.evaluate(() => window.issueNotifications.length) === 1);
  const notice = await page.evaluate(() => ({ title: window.issueNotifications[0].title, options: window.issueNotifications[0].options }));
  assert.equal(notice.title, "Akra");
  assert.equal(notice.options.body, "이슈에 새 결정 요청이 있습니다. 이슈 탭에서 확인해 주세요.");
  assert.ok(!JSON.stringify(notice).includes(question) && !JSON.stringify(notice).includes(title));
  assert.equal(requests.filter((entry) => entry.method === "akra/kernel/query").length, beforeBackgroundReads);
  const otherWorkspace = "/work/other-app";
  await app.evaluate(({ ipcMain }, cwd) => {
    ipcMain.removeHandler("akra:choose-folder");
    ipcMain.handle("akra:choose-folder", () => cwd);
  }, otherWorkspace);
  await page.getByTestId("new-folder-thread").click();
  await page.getByTestId("home").locator(".home-pickers button").first().click();
  await page.getByRole("option", { name: "폴더 고르기…", exact: true }).click();
  await until(() => requests.filter(entry => entry.method === "akra/kernel/attach").at(-1)?.params.workspace === otherWorkspace,
    "the home folder attaches its own repository");
  await page.evaluate(() => window.issueNotifications[0].onclick());
  await panel.locator('[data-decision-id="background-decision"]').waitFor();
  assert.equal(requests.filter(entry => entry.method === "akra/kernel/attach").at(-1)?.params.workspace, workspace,
    "an explicit issue notification takes priority over the selected home folder");
  await capture("notification");
  assert.equal(await page.evaluate(() => window.issueNotifications.length), 1);
  await panel.getByTestId("issues-report-tab").click();
  const beforeRepeatedNotificationReads = requests.filter((entry) => entry.params.query?.type === "issue").length;
  await page.evaluate(() => window.issueNotifications[0].onclick());
  await until(async () => requests.filter((entry) => entry.params.query?.type === "issue").length > beforeRepeatedNotificationReads
    && await panel.locator('[data-decision-id="background-decision"]').isVisible(), "the same notification to reopen and refresh detail from the report");
  assert.equal(await panel.getByTestId("issues-list-tab").getAttribute("aria-pressed"), "true");
  const deliveryRequest = { ...requestDecision("decision-delivery"), kind: "risk_exception", gate: "delivery",
    question: "보호 경로와 변경량을 확인한 뒤 이 결과를 제출할까요?", recommended: null,
    basis: { ...requestDecision().basis, work_order_id: "work-delivery",
      delivery: { base_commit_sha: "a".repeat(40), source_commit_sha: "b".repeat(40) } },
    delivery_review: { changed_files: 51, changed_lines: 1_001, reasons: ["protected_path", "diff_too_large"] },
    options: [{ id: "approve", label: "결과 제출 승인", effect: "이 커밋 범위의 결과 제출을 승인합니다." },
      { id: "hold", label: "보류", effect: "결과를 제출하지 않고 기다립니다." }],
  };
  const deliveryCard = () => panel.locator('[data-testid="issue-decision-card"][data-decision-id="decision-delivery"][data-state="open"]');
  const refreshDelivery = async (source) => {
    const reads = requests.filter((entry) => entry.params.query?.type === "issue").length;
    await panel.getByTestId("issue-execution-refresh").click();
    await until(async () => requests.filter((entry) => entry.params.query?.type === "issue").length > reads
      && await panel.getByTestId("issues-detail").getAttribute("aria-busy") === "false"
      && await deliveryCard().getByTestId("issue-delivery-source").count() === 1
      && await deliveryCard().getByTestId("issue-delivery-source").innerText() === source, "the frozen delivery review");
  };
  const beforeDeliveryCommands = commandRequests("answer_issue_decision").length;
  const beforeDeliveryDialogs = await app.evaluate(() => globalThis.issueInterventionFixture.dialogs.length);
  openDecisions = [deliveryRequest]; revision++;
  await refreshDelivery("b".repeat(40));
  assert.match(await deliveryCard().innerText(), /사람 승인 대기/);
  assert.equal(await deliveryCard().getByTestId("issue-delivery-base").innerText(), "a".repeat(40));
  assert.deepEqual(await deliveryCard().getByTestId("issue-delivery-reason").allTextContents(), ["보호 경로 변경", "변경량 한도 초과"]);
  assert.equal(await deliveryCard().getByTestId("issue-delivery-files").innerText(), "51개");
  assert.equal(await deliveryCard().getByTestId("issue-delivery-lines").innerText(), "1,001줄 (추가 + 삭제)");
  await deliveryCard().locator('input[value="approve"]').check();
  await deliveryCard().getByTestId("issue-decision-note").fill("검토 중인 변경의 메모");
  queriedAt += 60_000; revision++;
  await refreshDelivery("b".repeat(40));
  assert.equal(await deliveryCard().locator('input[value="approve"]').isChecked(), true);
  assert.equal(await deliveryCard().getByTestId("issue-decision-note").inputValue(), "검토 중인 변경의 메모");
  const pastDelivery = { ...structuredClone(deliveryRequest), id: "delivery-history", state: "superseded" };
  history.unshift({ decision: fact(pastDelivery), request: pastDelivery,
    resolution_reason: { code: "basis_changed", summary: "제출 대상 커밋이 바뀌었습니다." } });
  // 같은 decision revision이라도 표시 범위가 바뀌면 지난 선택을 새 source에 재사용하지 않는다.
  openDecisions = [{ ...deliveryRequest, basis: { ...deliveryRequest.basis,
    delivery: { ...deliveryRequest.basis.delivery, source_commit_sha: "c".repeat(40) } } }]; revision++;
  await refreshDelivery("c".repeat(40));
  assert.equal(await deliveryCard().locator('input[value="approve"]').isChecked(), false);
  assert.equal(await deliveryCard().getByTestId("issue-decision-note").inputValue(), "");
  assert.equal(await deliveryCard().getByTestId("issue-decision-answer").isDisabled(), true);
  assert.equal(await panel.locator('[data-decision-id="delivery-history"]').getByTestId("issue-delivery-source").innerText(), "b".repeat(40));
  await deliveryCard().locator('input[value="approve"]').check();
  await deliveryCard().getByTestId("issue-decision-note").fill("새 범위를 검토하는 메모");
  openDecisions = [{ ...openDecisions[0], basis: { ...openDecisions[0].basis,
    delivery: { ...openDecisions[0].basis.delivery, base_commit_sha: "d".repeat(40) } },
    delivery_review: { changed_files: 1, changed_lines: null, reasons: ["binary_diff"] } }]; revision++;
  await refreshDelivery("c".repeat(40));
  assert.equal(await deliveryCard().getByTestId("issue-delivery-base").innerText(), "d".repeat(40));
  assert.equal(await deliveryCard().getByTestId("issue-delivery-lines").innerText(), "미확인 · 바이너리 포함");
  assert.equal(await deliveryCard().locator('input[value="approve"]').isChecked(), false);
  assert.equal(await deliveryCard().getByTestId("issue-decision-note").inputValue(), "");
  assert.equal(commandRequests("answer_issue_decision").length, beforeDeliveryCommands);
  assert.equal(await app.evaluate(() => globalThis.issueInterventionFixture.dialogs.length), beforeDeliveryDialogs);
  await capture("delivery-review");
  console.log("PASS frozen delivery reasons, size and commits; same-range drafts survive and changed ranges clear drafts");

  const arbiterRequest = { ...requestDecision("decision-arbiter-wait"), on_timeout: "arbiter" };
  const arbiterCard = () => panel.locator('[data-testid="issue-decision-card"][data-decision-id="decision-arbiter-wait"][data-state="open"]');
  const arbiterFinished = (event) => gateway.sendKernelEvent({ type: "recorded", sequence: 1_000 + event.revision,
    recorded_at: "2026-11-22T09:00:01Z", event: { kind: "issue_decision_arbiter_finished", at: "2026-11-22T09:00:01Z", ...event } });
  openDecisions = [arbiterRequest]; revision++;
  await panel.getByTestId("issue-execution-refresh").click();
  await arbiterCard().waitFor();
  assert.match(await arbiterCard().getByTestId("issue-decision-timeout").innerText(), /중재자가 검토합니다/);
  await arbiterCard().locator('input[value="approve"]').check();
  const arbiterNote = "중재 결과를 기다리며 검토한 메모";
  await arbiterCard().getByTestId("issue-decision-note").fill(arbiterNote);
  const beforeArbiterNotices = await page.evaluate(() => window.issueNotifications.length);
  const beforeArbiterCommands = commandRequests("answer_issue_decision").length;
  openDecisions = [{ ...arbiterRequest, arbiter_wait: { reason: "low_confidence" } }]; revision++;
  queriedAt = Date.parse("2026-11-22T09:00:01Z");
  const waitingEvent = { event_id: "arbiter-wait-finished", revision, decision: fact(arbiterRequest), outcome: { kind: "waiting", reason: "low_confidence" } };
  arbiterFinished(waitingEvent);
  arbiterFinished(waitingEvent);
  await until(async () => await arbiterCard().getByTestId("issue-arbiter-wait").count() === 1
    && await arbiterCard().getByTestId("issue-decision-answer").isEnabled()
    && await page.evaluate(() => window.issueNotifications.length) === beforeArbiterNotices + 1, "current arbiter Wait and one notification");
  assert.match(await arbiterCard().innerText(), /사람 답변 대기/);
  assert.match(await arbiterCard().getByTestId("issue-arbiter-wait").innerText(), /판단 확신이 기준에 미치지 못했습니다/);
  assert.equal(await arbiterCard().locator('input[value="approve"]').isChecked(), true);
  assert.equal(await arbiterCard().getByTestId("issue-decision-note").inputValue(), arbiterNote);
  assert.match(await arbiterCard().getByTestId("issue-decision-timeout").innerText(), /사람의 답변을 기다립니다/);
  const waitNotice = await page.evaluate(() => window.issueNotifications.at(-1).options);
  assert.equal(waitNotice.body, "이슈 결정에 사람의 답변이 필요합니다. 이슈 탭에서 확인해 주세요.");
  assert.ok(!JSON.stringify(waitNotice).includes(question) && !JSON.stringify(waitNotice).includes(arbiterNote));
  await capture("arbiter-wait");

  const beforeWaitRefresh = requests.filter((entry) => entry.params.query?.type === "issue").length;
  arbiterFinished(waitingEvent);
  await panel.getByTestId("issue-execution-refresh").click();
  await until(async () => requests.filter((entry) => entry.params.query?.type === "issue").length > beforeWaitRefresh
    && await arbiterCard().getByTestId("issue-decision-answer").isEnabled(), "Wait persists after acknowledged-event replay and reread");
  assert.equal(await arbiterCard().locator('input[value="approve"]').isChecked(), true);
  assert.equal(await arbiterCard().getByTestId("issue-decision-note").inputValue(), arbiterNote);
  assert.equal(await page.evaluate(() => window.issueNotifications.length), beforeArbiterNotices + 1);
  assert.equal(commandRequests("answer_issue_decision").length, beforeArbiterCommands);

  await app.evaluate(() => { globalThis.issueInterventionFixture.nextAnswer = 0; });
  await arbiterCard().getByTestId("issue-decision-answer").click();
  const answeredArbiter = panel.locator('[data-testid="issue-decision-card"][data-decision-id="decision-arbiter-wait"][data-state="answered"]');
  await until(async () => commandRequests("answer_issue_decision").length === beforeArbiterCommands + 1
    && await answeredArbiter.getByTestId("issue-decision-origin").count() === 1, "a confirmed Human answer remains possible after arbiter Wait");
  assert.equal(commandRequests("answer_issue_decision").at(-1).request.command.note, arbiterNote);
  assert.equal(commandRequests("answer_issue_decision").at(-1).method, "akra/kernel/command_confirmed");
  assert.match(await answeredArbiter.getByTestId("issue-decision-origin").innerText(), /사람이 답변했습니다/);
  assert.equal(await answeredArbiter.getByTestId("issue-arbiter-wait").count(), 0);
  const beforeLateWaitReads = requests.filter((entry) => entry.params.query?.type === "issue").length;
  arbiterFinished({ ...waitingEvent, event_id: "late-arbiter-wait" });
  await until(async () => requests.filter((entry) => entry.params.query?.type === "issue").length > beforeLateWaitReads
    && await panel.getByTestId("issues-detail").getAttribute("aria-busy") === "false"
    && await answeredArbiter.getByTestId("issue-decision-origin").count() === 1, "a late Waiting fact rechecks the Human history");
  assert.equal(await arbiterCard().count(), 0);
  assert.equal(await page.evaluate(() => window.issueNotifications.length), beforeArbiterNotices + 1);

  await page.getByTestId("issues-toggle").click();
  await panel.waitFor({ state: "detached" });
  const appliedRequest = { ...requestDecision("decision-arbiter-applied"), on_timeout: "arbiter", state: "answered" };
  const appliedFact = { ...fact(appliedRequest), answer_by: "arbiter", option: "approve" };
  history.unshift({ decision: appliedFact, request: appliedRequest, resolution_reason: { code: "answered", summary: "중재자의 답변을 기록했습니다." } });
  revision++;
  const appliedEvent = { event_id: "arbiter-applied-finished", revision, decision: appliedFact, outcome: { kind: "applied" } };
  arbiterFinished(appliedEvent);
  arbiterFinished(appliedEvent);
  await until(async () => await page.evaluate(() => window.issueNotifications.length) === beforeArbiterNotices + 2, "one verified Applied notification while the issue panel is closed");
  assert.equal(await page.evaluate(() => window.issueNotifications.at(-1).options.body), "중재자가 이슈 결정에 답변했습니다. 이슈 탭에서 확인해 주세요.");
  await page.evaluate(() => window.issueNotifications.at(-1).onclick());
  const appliedCard = panel.locator('[data-testid="issue-decision-card"][data-decision-id="decision-arbiter-applied"][data-state="answered"]');
  await appliedCard.waitFor();
  assert.match(await appliedCard.getByTestId("issue-decision-origin").innerText(), /중재자가 답변했습니다/);
  assert.match(await answeredArbiter.getByTestId("issue-decision-origin").innerText(), /사람이 답변했습니다/);
  assert.equal(await appliedCard.getByTestId("issue-decision-answer").count(), 0);
  assert.equal(await page.evaluate(() => window.issueNotifications.length), beforeArbiterNotices + 2);
  await capture("arbiter-history");
  console.log("PASS current arbiter Wait preserves drafts and Human confirmation; late Wait cannot reopen; Applied history and notifications retain original actor");

  const criteriaRequest = { ...requestDecision("decision-criteria-adoption"),
    options: [{ id: "start", label: "작업 시작", effect: "확정한 완료 기준으로 작업을 생성합니다." }],
    recommended: "start", criteria_scenarios: ["검색 결과를 그대로 보존한다."],
    basis: { ...requestDecision().basis, field_hash: { version: 1, sha256: "a".repeat(64) } } };
  openDecisions = [criteriaRequest]; revision++;
  await panel.getByTestId("issue-execution-refresh").click();
  const criteriaCard = () => panel.locator('[data-testid="issue-decision-card"][data-decision-id="decision-criteria-adoption"][data-state="open"]');
  await criteriaCard().waitFor();
  await criteriaCard().locator('input[value="start"]').check();
  assert.equal(await criteriaCard().getByTestId("issue-decision-answer").isDisabled(), true);
  assert.equal(await criteriaCard().getByTestId("task-criterion").count(), 0, "the source text does not infer a validation channel");
  await criteriaCard().getByLabel("제안 기준 1 검증 채널").selectOption("command");
  await criteriaCard().getByLabel("호스트 검증").selectOption("git_diff_check_v1");
  const beforeCriteria = commandRequests("answer_issue_decision").length;
  await app.evaluate(() => { globalThis.issueInterventionFixture.nextAnswer = 1; });
  await criteriaCard().getByTestId("issue-decision-answer").click();
  await until(async () => await criteriaCard().getByTestId("issue-decision-answer").isEnabled());
  assert.equal(commandRequests("answer_issue_decision").length, beforeCriteria, "cancelled native review writes nothing");
  assert.equal(await criteriaCard().getByTestId("task-criterion").count(), 1, "cancellation preserves the user's exact criteria draft");
  assert.equal(await criteriaCard().getByLabel("호스트 검증").inputValue(), "git_diff_check_v1");
  openDecisions = [{ ...criteriaRequest, basis: { ...criteriaRequest.basis, field_hash: { version: 1, sha256: "b".repeat(64) } } }];
  revision++;
  await panel.getByTestId("issue-execution-refresh").click();
  await until(async () => await criteriaCard().locator('input[value="start"]').isChecked() === false);
  await criteriaCard().locator('input[value="start"]').check();
  assert.equal(await criteriaCard().getByTestId("task-criterion").count(), 0, "a changed issue specification discards criteria tied to the previous approval basis");
  await criteriaCard().getByLabel("제안 기준 1 검증 채널").selectOption("command");
  await criteriaCard().getByLabel("호스트 검증").selectOption("git_diff_check_v1");
  await capture("criteria-adoption");
  await app.evaluate(() => { globalThis.issueInterventionFixture.nextAnswer = 0; });
  await criteriaCard().getByTestId("issue-decision-answer").click();
  await until(async () => commandRequests("answer_issue_decision").length === beforeCriteria + 1);
  assert.equal(commandRequests("answer_issue_decision").at(-1).method, "akra/kernel/command_confirmed");
  assert.deepEqual(commandRequests("answer_issue_decision").at(-1).request.command.criteria, [
    { id: "issue-criterion-1", scenario: "검색 결과를 그대로 보존한다.", channel: "command", essential: true, verification_recipe: "git_diff_check_v1" },
  ]);
  console.log("PASS explicit issue criteria → native review → confirmed submission, cancellation and stale-spec draft fencing");

  assert.equal(gateway.receivedMethods.includes("turn/start"), false);
  assert.equal(gateway.receivedMethods.includes("turn/steer"), false);
  assert.deepEqual(pageErrors, []);
  console.log("PASS background-only fixed-text notification, deduplication and fresh issue navigation");
} catch (error) {
  if (page && !page.isClosed()) {
    const shown = await page.evaluate(() => ({
      connection: document.querySelector('[data-testid="connection-phase"]')?.textContent,
      panel: document.querySelector('[data-testid="issues-panel"]')?.textContent,
    })).catch(() => null);
    console.error("issue intervention fixture failure", JSON.stringify({ revision, level, cap, shown }));
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
    if (!closed) { appProcess.kill(); throw new Error("hidden fixture application did not close"); }
  }
  await gateway.close();
  await rm(userData, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 });
}
