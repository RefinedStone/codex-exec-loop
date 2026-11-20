// #2436: hidden Electron + 가짜 Gateway. GitHub 쓰기/모델 호출 없이 생성 의도 재개와 승인 경계를 검증한다.
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { _electron as electron } from "playwright-core";
import { exerciseReadiness, readinessFixture } from "./fixtures/issue-readiness.mjs";

const issueScreenshotPath = process.env.AKRA_QA_ISSUE_CREATE_SCREENSHOT ?? process.env.AKRA_QA_SCREENSHOT;

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = process.env.AKRA_E2E_ELECTRON_EXECUTABLE ?? createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);
const SECRET = "synthetic-create-secret-0000";
const LOG = "synthetic-create-log-1111";
const target = { cwd: "/work/example-app", owner: "sample", repository: "widgets" };
const repository = { provider: "github", serverId: "github.com", repositoryId: "R_widgets", projectScope: { workspaceDir: target.cwd } };
const resultUrl = "https://github.com/sample/widgets/issues/501";
const hash = (copy) => ({ version: 1, sha256: createHash("sha256").update(JSON.stringify(copy)).digest("hex") });
const clone = (value) => structuredClone(value);
const readiness = readinessFixture();
const issue = { policy: "p1", intent: null, approval: null, sequence: 0, outcome: "held", creates: 0, loseNextRestartReply: false, requests: [], violations: [] };

function newIntent(predecessorIntentId = null) {
  issue.approval = null;
  issue.intent = {
    intentId: "create-" + ++issue.sequence, state: "prepared", created: null, sharedCopy: null,
    restoreLinkRequired: false, restoreLinkFinalized: false, predecessorIntentId,
  };
}
function opened() {
  if (!issue.intent) newIntent();
  const recoveryOnly = ["sent", "needs_confirmation", "confirmed", "abandoned"].includes(issue.intent.state);
  return clone({
    repository, intent: issue.intent,
    title: issue.intent.sharedCopy?.title ?? "", body: issue.intent.sharedCopy?.body ?? "## Steps\n",
    template: recoveryOnly
      ? { status: "recovery_only", path: null, reason: null, requiredInputs: [], unsupportedInputs: [] }
      : { status: "resolved", path: ".github/ISSUE_TEMPLATE/bug.md", reason: null, requiredInputs: [{ label: "Steps", kind: "section", options: [] }], unsupportedInputs: [] },
    requiredSections: recoveryOnly ? [] : ["Impact"],
    spanActions: { secret: "remove", name: "restorable_mask", path: "restorable_mask", log: "restorable_mask" },
  });
}
function makePreview(text) {
  const copy = { title: text.title, body: text.body };
  const markers = [];
  for (const [index, span] of text.spans.map((span, index) => [index, span]).sort((a, b) => b[1].start - a[1].start)) {
    const mask = span.decision === "restorable_mask" || (span.decision === null && span.class === "log");
    const marker = "[[masked:" + (index + 1) + "]]";
    if (mask) markers.unshift({ marker, field: span.field });
    copy[span.field] = copy[span.field].slice(0, span.start) + (mask ? marker : "[removed]") + copy[span.field].slice(span.end);
  }
  const errors = [
    ...(!copy.title.trim() ? [{ field: "title", label: null, kind: "empty" }] : []),
    ...(!copy.body.includes("## Impact") ? [{ field: "required_section", label: "Impact", kind: "missing" }] : []),
  ];
  const draftApproval = { sharedCopyHash: hash(copy), policyVersion: issue.policy, templateVersion: "t1" };
  return {
    preview: { status: "ready", sharedCopy: copy, markers, errors, validatorAbsent: false, publishable: errors.length === 0, approval: draftApproval },
    approval: { ...draftApproval, repository: clone(repository), intentId: issue.intent.intentId },
  };
}
function frozenPreview(text) {
  if (JSON.stringify({ title: text.title, body: text.body }) !== JSON.stringify(issue.intent.sharedCopy) || text.spans.length !== 0) {
    issue.violations.push("frozen preview replaced the approved shared copy");
  }
  const result = makePreview({ ...issue.intent.sharedCopy, spans: [] });
  result.approval = issue.approval.policyVersion === issue.policy ? clone(issue.approval) : null;
  return result;
}
function answerCreate(method, params) {
  issue.requests.push(clone({ method, params }));
  if (method === "akra/issue/create/open") return opened();
  if (method === "akra/issue/create/restart") {
    if (params.previousIntentId !== issue.intent.intentId) throw new Error("현재 생성 의도가 아닙니다.");
    if (["sent", "needs_confirmation"].includes(issue.intent.state) && !params.acknowledgeUncertain) throw new Error("중복 가능성 확인이 필요합니다.");
    const predecessor = issue.intent.intentId;
    newIntent(predecessor);
    if (issue.loseNextRestartReply) {
      issue.loseNextRestartReply = false;
      throw new Error("새 생성 의도를 저장한 뒤 응답을 읽지 못했습니다.");
    }
    return opened();
  }
  if (method === "akra/issue/readinessBarrier") return { ok: true };
  if (params.intentId !== issue.intent.intentId) throw new Error("현재 생성 의도가 아닙니다.");
  if (method === "akra/issue/create/preview") {
    const frozen = issue.intent.state !== "prepared";
    const result = frozen ? frozenPreview(params.text) : makePreview(params.text);
    return Promise.resolve(readiness.reply(result.preview, frozen)).then((preview) => ({ ...result, preview }));
  }
  if (method === "akra/issue/create/publish") {
    const prepared = issue.intent.state === "prepared";
    if (!prepared && !["held", "recorded"].includes(issue.intent.state)) return { status: "refused", reason: "intent_requires_confirmation" };
    const result = prepared ? makePreview(params.text) : frozenPreview(params.text);
    if (!result.preview.publishable) return { status: "blocked", preview: result };
    if (!result.approval || JSON.stringify(params.approval) !== JSON.stringify(result.approval)) return { status: "refused", reason: "approval_mismatch" };
    if (prepared) {
      issue.intent.sharedCopy = clone(result.preview.sharedCopy);
      issue.approval = clone(params.approval);
    } else if (!params.sharedCopyOnly) {
      issue.violations.push("a frozen intent was sent without explicit shared-only consent");
      return { status: "held", intent: clone(issue.intent) };
    }
    if (prepared && (issue.outcome === "held" || issue.outcome === "recorded")) {
      issue.intent.state = issue.outcome;
      return { status: "held", intent: clone(issue.intent) };
    }
    issue.creates += 1;
    issue.intent.state = issue.outcome === "lost-with-recovery" ? "sent" : "needs_confirmation";
    issue.intent.restoreLinkRequired = !params.sharedCopyOnly && issue.outcome === "lost-with-recovery";
    if (issue.outcome === "lost-with-recovery") throw new Error("생성 요청 뒤 응답을 읽지 못했습니다.");
    return { status: "needs_confirmation", intent: clone(issue.intent) };
  }
  if (method === "akra/issue/create/recheck") {
    if (issue.intent.state === "sent") issue.intent.state = "needs_confirmation";
    if (issue.intent.state === "confirmed") issue.intent.restoreLinkFinalized = true;
    return clone(issue.intent);
  }
  if (method === "akra/issue/create/confirm") {
    if (params.url !== resultUrl) throw new Error("승인한 저장소 또는 공유본과 일치하지 않는 URL입니다.");
    issue.intent.state = "confirmed";
    issue.intent.created = { issue: { ...clone(repository), issueId: "I_501" }, locator: { owner: "sample", repository: "widgets", number: 501 }, url: resultUrl };
    return clone(issue.intent);
  }
  throw new Error("알 수 없는 생성 메서드");
}

const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], { issue: answerCreate });
const userData = await mkdtemp(join(tmpdir(), "akra-issue-create-ui-"));
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;
const failures = [];
const check = (name, ok) => {
  console.log((ok ? "PASS " : "FAIL ") + name);
  if (!ok) failures.push(name);
};
const until = async (condition) => {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    if (await condition()) return true;
    await new Promise((resolve) => setTimeout(resolve, 80));
  }
  return false;
};
const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  await app.evaluate(({ ipcMain }) => {
    for (const channel of ["akra:github-summary", "akra:github-project", "akra:github-repository"]) ipcMain.removeHandler(channel);
    ipcMain.handle("akra:github-summary", () => ({ login: "sample", myPullRequests: [], reviewRequests: [] }));
    ipcMain.handle("akra:github-repository", (_event, cwd) => cwd === "/work/example-app" ? "sample/widgets" : null);
    ipcMain.handle("akra:github-project", () => ({
      title: "Sample board", url: "https://github.com/users/sample/projects/1",
      items: [{ kind: "issue", title: "Issue 7", url: "https://github.com/sample/widgets/issues/7", repository: "sample/widgets", number: 7, status: "Todo" }],
      closedCount: 0, truncated: false,
    }));
  });
  const page = await app.firstWindow();
  const consoleMessages = [];
  page.on("console", (message) => consoleMessages.push(message.text()));
  const panel = page.getByTestId("issue-create-panel");
  const title = panel.getByTestId("issue-draft-title");
  const body = panel.getByTestId("issue-draft-body");
  const approve = panel.getByTestId("issue-create-approve");
  const status = panel.getByTestId("issue-create-status");
  const publishCount = () => issue.requests.filter(({ method }) => method === "akra/issue/create/publish").length;
  const statusIs = (state) => until(async () => (await status.count()) > 0 && (await status.getAttribute("data-status")) === state);
  const openWork = async () => {
    await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
    await page.getByTestId("thread-entry").first().click();
    await page.getByTestId("work-toggle").click();
    await page.getByTestId("github-toggle").click();
  };
  const openGitHub = async () => {
    const entry = page.getByTestId("github-create-issue");
    if (!await until(() => entry.isEnabled())) throw new Error("새 이슈 진입이 활성화되지 않았습니다.");
    await entry.click();
    await status.waitFor();
  };
  const mark = async (value, kind) => {
    await body.evaluate((element, wanted) => {
      const start = element.value.indexOf(wanted);
      element.focus();
      element.setSelectionRange(start, start + wanted.length);
    }, value);
    await panel.getByTestId("issue-draft-span-class").selectOption(kind);
    await panel.getByTestId("issue-draft-mark-body").click();
  };
  const approveAndPublish = async () => {
    if (!await until(() => approve.isEnabled())) throw new Error("생성 승인이 활성화되지 않았습니다.");
    await approve.check();
    await panel.getByTestId("issue-create-publish").click();
  };
  const fillValid = async (heading, withLog = false) => {
    await title.fill(heading);
    await body.fill("## Steps\n" + (withLog ? LOG + "\n" : "재현\n") + "## Impact\n영향\n");
    if (withLog) await mark(LOG, "log");
  };

  await openWork();
  await openGitHub();
  check("생성 대상에는 저장소와 cwd만 있고 아직 이슈 번호가 없다", JSON.stringify(issue.requests[0].params) === JSON.stringify(target));
  const requirements = await panel.getByTestId("issue-draft-requirements").innerText();
  check("생성 화면도 템플릿 필드와 필수 섹션을 보여 준다", requirements.includes('"Steps"') && requirements.includes('"Impact"'));
  await title.fill("새 오류");
  await body.fill("## Steps\n" + SECRET + "\n" + LOG + "\n");
  check("필수 섹션을 채우기 전에는 승인할 수 없다", await until(async () => (await panel.getByTestId("issue-draft-errors").count()) > 0 && await approve.isDisabled()));
  await mark(SECRET, "secret");
  await mark(LOG, "log");
  await body.fill("## Steps\n" + SECRET + "\n" + LOG + "\n## Impact\n영향\n");
  check("공유본은 제거와 복원 표식을 반영한다", await until(async () => {
    const text = await panel.getByTestId("issue-draft-shared-body").innerText();
    return text.includes("[removed]") && text.includes("[[masked:2]]") && !text.includes(SECRET) && !text.includes(LOG);
  }));
  await exerciseReadiness({
    page, panel, fixture: readiness, requests: issue.requests, previewMethod: "akra/issue/create/preview",
    approval: approve, publish: panel.getByTestId("issue-create-publish"), errorId: "issue-create-error", until, check,
  });
  if (issueScreenshotPath) {
    await panel.getByTestId("issue-draft-readiness").scrollIntoViewIfNeeded();
    await page.screenshot({ path: issueScreenshotPath, animations: "disabled" });
  }
  await approve.check();
  await title.fill("수정한 새 오류");
  check("원문을 고치면 다음 미리보기 전에도 승인이 즉시 취소된다", !(await approve.isChecked()) && await panel.getByTestId("issue-create-publish").isDisabled());
  await until(() => approve.isEnabled());
  await approve.check();
  issue.policy = "p2";
  await panel.getByTestId("issue-create-publish").click();
  check("현재 정책과 다른 승인은 서버 거절 뒤 다시 승인하게 한다", await until(async () => (await panel.getByTestId("issue-create-notice").count()) > 0 && !(await approve.isChecked())));
  check("거절된 승인은 새 이슈를 만들지 않는다", issue.creates === 0 && issue.intent.state === "prepared");
  await approveAndPublish();
  check("복원 자료를 기록하지 못한 생성은 Held로 고정된다", await statusIs("held"));
  check("Held는 원문 편집기를 닫고 승인한 공유본만 보여 준다", (await title.count()) === 0 && (await panel.getByTestId("issue-create-frozen-body").innerText()).includes("[[masked:2]]"));
  check("Held의 원문은 화면에서 제거된다", !(await panel.innerText()).includes(SECRET) && !(await panel.innerText()).includes(LOG));
  await panel.getByTestId("issue-draft-preview").waitFor();
  check("고정된 게시본을 새 준비검사 입력으로 대체하지 않는다",
    await until(async () => (await panel.getByTestId("issue-draft-readiness-blocked").count()) === 1 &&
      (await panel.getByTestId("issue-draft-readiness-blocked").getAttribute("data-reason")) === "frozen_draft") &&
    await panel.getByTestId("issue-draft-expected-labels").isDisabled());
  check("고정된 공유본은 원문 구간 정보 없이 표식 0개로 표시하지 않는다", !(await panel.innerText()).includes("표식이 없습니다.") && !(await panel.innerText()).includes("남는 가림 표식"));
  const firstIntent = issue.intent.intentId;
  const beforeResume = publishCount();
  await panel.getByTestId("issue-create-close").click();

  await page.getByTestId("projects-toggle").click();
  const project = page.getByTestId("github-project-panel");
  await project.getByLabel("Project", { exact: true }).fill("sample/1");
  await project.getByRole("button", { name: "저장", exact: true }).click();
  await project.getByTestId("github-project-filter").waitFor();
  await project.getByTestId("github-project-filter").selectOption("*");
  check("Project 전체 보기에서는 생성 저장소를 먼저 골라야 한다", await project.getByTestId("github-project-create").isDisabled());
  await project.getByTestId("github-project-filter").selectOption("sample/widgets");
  await project.getByTestId("github-project-create").click();
  check("Project에서 다시 열어도 같은 Held 의도이며 재게시하지 않는다", await statusIs("held") && issue.intent.intentId === firstIntent && publishCount() === beforeResume);
  issue.policy = "p3";
  await panel.getByTestId("issue-create-close").click();
  await project.getByTestId("github-project-create").click();
  check("고정된 승인과 현재 정책이 다르면 공유본 재개를 막는다", await until(async () => (await panel.getByTestId("issue-create-frozen-policy").count()) > 0 && await approve.isDisabled()));
  await panel.getByTestId("issue-create-restart").click();
  check("명시적 새 초안만 새로운 생성 의도를 만든다", await statusIs("prepared") && issue.intent.intentId !== firstIntent && issue.intent.predecessorIntentId === firstIntent);
  check("새 초안은 이전 시도와 별도 이슈를 만든다는 관계를 표시한다", (await panel.getByTestId("issue-create-predecessor").innerText()).includes("별도의 새 이슈"));
  const restartOne = issue.requests.filter(({ method }) => method === "akra/issue/create/restart").at(-1);
  check("아직 보내지 않은 의도의 교체는 불확실 승인 없이 요청한다", restartOne.params.previousIntentId === firstIntent && restartOne.params.acknowledgeUncertain === false);

  issue.outcome = "recorded";
  await fillValid("기록된 생성");
  await approveAndPublish();
  check("Recorded도 기존 공유본을 고정하여 재개한다", await statusIs("recorded") && (await title.count()) === 0);
  check("공유본만 게시하려면 별도 명시적 승인이 필요하다", !(await approve.isChecked()) && await panel.getByTestId("issue-create-shared-only").isDisabled());
  await until(() => approve.isEnabled());
  await approve.check();
  await panel.getByTestId("issue-create-shared-only").click();
  check("공유본만 게시한 뒤에는 결과 확인 상태로 바뀐다", await statusIs("needs_confirmation") && issue.creates === 1);
  const sharedRequest = issue.requests.filter(({ method }) => method === "akra/issue/create/publish").at(-1).params;
  check("공유본 재개는 같은 의도와 전체 저장소 승인을 보낸다", sharedRequest.sharedCopyOnly && sharedRequest.intentId === issue.intent.intentId && JSON.stringify(sharedRequest.approval.repository) === JSON.stringify(repository));
  const beforeReload = publishCount();
  const sentIntent = issue.intent.intentId;
  await page.reload();
  await openWork();
  await openGitHub();
  check("창 재시작은 이전 의도를 재개하고 create를 자동 재시도하지 않는다", await statusIs("needs_confirmation") && issue.intent.intentId === sentIntent && publishCount() === beforeReload && issue.creates === 1);
  check("정책을 읽지 않는 복구 전용 화면임을 안내한다", (await panel.getByTestId("issue-draft-recovery-only").innerText()).includes("기록된 생성 요청의 결과"));
  check("확인 필요 상태에는 생성·편집 버튼이 없다", (await panel.getByTestId("issue-create-publish").count()) === 0 && (await title.count()) === 0);
  await panel.getByTestId("issue-create-recheck").click();
  check("확인 조회는 create를 추가하지 않는다", await statusIs("needs_confirmation") && publishCount() === beforeReload && issue.creates === 1);
  await panel.getByTestId("issue-create-result-url").fill("https://github.com/other/widgets/issues/501");
  await panel.getByTestId("issue-create-confirm").click();
  check("일치하지 않는 URL은 확인 실패로 남는다", await until(async () => (await panel.getByTestId("issue-create-error").count()) > 0) && issue.intent.state === "needs_confirmation");
  await panel.getByTestId("issue-create-result-url").fill(resultUrl);
  await panel.getByTestId("issue-create-confirm").click();
  check("정확한 URL 확인 뒤에는 생성 확인으로 표시한다", await statusIs("confirmed") && (await panel.getByTestId("issue-create-open-result").count()) === 1);

  await panel.getByTestId("issue-create-restart").click();
  await statusIs("prepared");
  issue.outcome = "lost-with-recovery";
  await fillValid("복원 연결 있는 생성", true);
  await approveAndPublish();
  check("보낸 뒤 응답 오류는 결과 미확정으로 잠근다", await until(async () => (await panel.getByTestId("issue-create-reopen").count()) > 0) && await statusIs("needs_confirmation"));
  const afterLost = publishCount();
  await panel.getByTestId("issue-create-reopen").click();
  check("Sent 기록 재조회도 생성 요청을 반복하지 않는다", await until(async () => (await status.getAttribute("data-status")) === "needs_confirmation" && (await panel.getByTestId("issue-create-result-url").count()) === 1) && publishCount() === afterLost);
  check("Sent도 정책 재검사 없이 복구 전용으로 다시 열린다", (await panel.getByTestId("issue-draft-recovery-only").count()) === 1);
  await panel.getByTestId("issue-create-result-url").fill(resultUrl);
  await panel.getByTestId("issue-create-confirm").click();
  check("생성 완료와 복원 연결 대기를 별도 상태로 보여 준다", await statusIs("restore_pending") && (await status.innerText()).includes("생성 완료 · 복원 연결 대기"));
  const restoreCreates = issue.creates;
  await panel.getByTestId("issue-create-recheck").click();
  check("복원 연결 재확인은 create를 반복하지 않고 완료된다", await statusIs("confirmed") && issue.intent.restoreLinkFinalized && issue.creates === restoreCreates && publishCount() === afterLost);

  await panel.getByTestId("issue-create-restart").click();
  await statusIs("prepared");
  issue.outcome = "send";
  await fillValid("불확실 요청의 명시적 교체");
  await approveAndPublish();
  await statusIs("needs_confirmation");
  const uncertainId = issue.intent.intentId;
  check("불확실 생성의 새 초안은 중복 가능성 확인 전에는 막힌다", await panel.getByTestId("issue-create-restart").isDisabled());
  await panel.getByTestId("issue-create-restart-ack").check();
  const restartCount = issue.requests.filter(({ method }) => method === "akra/issue/create/restart").length;
  const beforeLostRestart = { sequence: issue.sequence, publishes: publishCount(), creates: issue.creates };
  issue.loseNextRestartReply = true;
  await panel.getByTestId("issue-create-restart").click();
  check("새 초안 저장 뒤 응답을 잃으면 생성 기록 재조회로 복구하게 한다",
    await until(async () => (await panel.getByTestId("issue-create-reopen").count()) === 1) &&
    issue.sequence === beforeLostRestart.sequence + 1);
  const successorId = issue.intent.intentId;
  await panel.getByTestId("issue-create-reopen").click();
  check("응답을 잃은 재시작은 저장된 새 의도를 다시 열어 복구한다",
    await statusIs("prepared") && issue.intent.intentId === successorId && issue.intent.predecessorIntentId === uncertainId);
  check("재시작 복구는 새 의도를 중복 저장하거나 생성 POST를 추가하지 않는다",
    issue.sequence === beforeLostRestart.sequence + 1 &&
    issue.requests.filter(({ method }) => method === "akra/issue/create/restart").length === restartCount + 1 &&
    publishCount() === beforeLostRestart.publishes && issue.creates === beforeLostRestart.creates);
  const finalRestart = issue.requests.filter(({ method }) => method === "akra/issue/create/restart").at(-1).params;
  check("불확실 교체 요청에 사용자 확인이 전달된다", finalRestart.previousIntentId === uncertainId && finalRestart.acknowledgeUncertain === true);

  const storage = await page.evaluate(() => JSON.stringify({ ...localStorage }) + JSON.stringify({ ...sessionStorage }));
  const composer = page.getByTestId("composer-input");
  check("원문을 브라우저 저장소에 남기지 않는다", !storage.includes(SECRET) && !storage.includes(LOG));
  check("원문을 대화 입력창이나 콘솔에 넣지 않는다", ((await composer.count()) === 0 || !(await composer.inputValue()).includes(SECRET)) && !consoleMessages.some((text) => text.includes(SECRET) || text.includes(LOG)));
  check("고정된 공유본과 승인 계약을 유지한다", issue.violations.length === 0);
  check("모델 턴을 시작하지 않는다", !gateway.receivedMethods.includes("turn/start") && !gateway.receivedMethods.includes("turn/steer"));
  await panel.getByTestId("issue-create-close").click();
  check("닫으면 GitHub 작업 목록으로 돌아간다", await until(async () => (await page.getByTestId("github-panel").count()) > 0));
} catch (error) {
  check("화면 흐름 완료: " + String(error).split("\n")[0], false);
} finally {
  const exited = new Promise((resolve) => app.process().once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  await gateway.close();
  await rm(userData, { recursive: true, force: true });
}
process.exit(failures.length === 0 ? 0 : 1);
