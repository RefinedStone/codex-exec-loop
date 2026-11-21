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
  ], recommended: "approve", on_timeout: "wait",
});
let openDecisions = [requestDecision()];
const fact = (request, state = request.state) => ({
  decision_id: request.id, kind: request.kind, subject: request.subject, gate: request.gate, basis: request.basis, state,
});
let history = [{ decision: fact(requestDecision("prior-withheld"), "superseded"), resolution_reason: { code: "basis_changed", summary: "판정 기준이 바뀌어 이전 결정을 종료했습니다." } }];
const rank = (value) => ["off", "observe", "propose", "gated", "auto"].indexOf(value);
const autonomy = () => ({
  status: "ready", requested_level: level, effective_level: effectiveOverride ?? level, cap,
  revision, policy_version: "runtime-policy-" + revision, pending: false,
  choices: ["off", "observe", "propose"].map((choice) => ({
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
const detail = () => ({ type: "issue", status: "ready", revision, item: item(), open_decisions: structuredClone(openDecisions),
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
      revision++;
      const resolved = supersedeAnswer ? "superseded" : "answered";
      const decision = { ...fact(current, resolved), option: supersedeAnswer ? null : command.option, answer_by: supersedeAnswer ? null : "human" };
      history.unshift({ decision, request: { ...current, state: resolved }, resolution_reason: { code: supersedeAnswer ? "basis_changed" : "answered",
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
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("issues-toggle").click();
  const panel = page.getByTestId("issues-panel");
  const select = panel.getByTestId("issues-autonomy-level");
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
  cap = "observe"; level = "observe"; revision++;
  gateway.sendKernelEvent(state("issues"));
  await until(async () => await select.inputValue() === "observe" && await select.locator('option[value="propose"]').evaluate((option) => option.disabled), "the pushed lower autonomy cap");
  assert.match(await panel.getByTestId("issues-autonomy-reason").innerText(), /상한/);
  cap = "auto"; level = null; effectiveOverride = "auto"; revision++;
  gateway.sendKernelEvent(state("issues"));
  await until(async () => await select.inputValue() === "auto", "the policy-owned Auto level");
  assert.match(await panel.getByTestId("issues-autonomy-effective").innerText(), /자동/);
  await capture("autonomy");
  cap = "propose"; level = "propose"; effectiveOverride = null; revision++;
  gateway.sendKernelEvent(state("issues"));
  await until(async () => await select.inputValue() === "propose");
  console.log("PASS native autonomy confirmation, cap reasons and truthful policy Auto display");

  await panel.getByTestId("issues-title").click();
  const currentCard = () => panel.locator('[data-testid="issue-decision-card"][data-state="open"]').first();
  await currentCard().waitFor();
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
  await currentCard().getByTestId("issue-decision-note").fill("공유 가능한 검증 메모입니다.");
  await app.evaluate(() => { globalThis.issueInterventionFixture.nextAnswer = 1; });
  await currentCard().getByTestId("issue-decision-answer").click();
  await until(async () => await currentCard().getByTestId("issue-decision-answer").isEnabled());
  assert.equal(commandRequests("answer_issue_decision").length, 0);
  assert.equal(await currentCard().locator('input[value="approve"]').isChecked(), true);
  assert.equal(await currentCard().getByTestId("issue-decision-note").inputValue(), "공유 가능한 검증 메모입니다.");
  await app.evaluate(() => { globalThis.issueInterventionFixture.nextAnswer = 0; });
  await currentCard().getByTestId("issue-decision-answer").click();
  await until(async () => commandRequests("answer_issue_decision").length === 1 && (await panel.locator('[data-testid="issue-decision-card"][data-state="open"]').count()) === 0);
  assert.equal(commandRequests("answer_issue_decision")[0].request.command.note, "공유 가능한 검증 메모입니다.");
  assert.equal(await panel.locator('[data-state="answered"] input').first().isDisabled(), true);
  openDecisions = [requestDecision("decision-stale")]; revision++;
  gateway.sendKernelEvent({ type: "issue_decision", decision: fact(openDecisions[0]), revision });
  await currentCard().waitFor();
  await currentCard().locator('input[value="approve"]').check();
  await app.evaluate(() => { globalThis.issueInterventionFixture.nextAnswer = null; });
  const dialogCount = await app.evaluate(() => globalThis.issueInterventionFixture.dialogs.length);
  await currentCard().getByTestId("issue-decision-answer").click();
  await until(async () => (await app.evaluate(() => globalThis.issueInterventionFixture.dialogs.length)) > dialogCount);
  supersedeAnswer = true;
  await app.evaluate(() => globalThis.issueInterventionFixture.resolve(0));
  await until(async () => (await panel.locator('[data-testid="issue-decision-card"][data-state="open"]').count()) === 0 && commandRequests("answer_issue_decision").length === 2);
  assert.match(await panel.getByTestId("issues-command-notice").innerText(), /이전 결정에 답하지 않았습니다/);
  const superseded = panel.locator('[data-testid="issue-decision-card"][data-state="superseded"]');
  assert.equal(await superseded.locator("input").first().isDisabled(), true);
  assert.match(await superseded.innerText(), /판정 기준이 바뀌어/);
  console.log("PASS issue-only cards, native cancellation, one answer and superseded disabled history");

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
  await page.evaluate(() => window.issueNotifications[0].onclick());
  await panel.locator('[data-decision-id="background-decision"]').waitFor();
  await capture("notification");
  assert.equal(await page.evaluate(() => window.issueNotifications.length), 1);
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
