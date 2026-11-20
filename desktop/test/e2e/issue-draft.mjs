// 화면 시험 (#2426): 빌드한 앱을 가짜 Gateway 에 붙여 이슈 초안 패널을 확인한다. gh, git, 실제 GitHub 쓰기는 없다.
// `akra/issue/*` 는 아래의 가짜가 답한다. 요구 사항과 검증 오류, 게시될 공유본과 남는 표식을 보고, 내용이나 정책이 바뀌면
// 승인이 지워지는지 보고, 게시 확인 / 게시 결과 확인 필요 / 알려진 외부 변경이 서로 다른 상태로 보이는지 본다.
// 로컬 글이 localStorage 와 대화 입력창에 가지 않는지도 본다. 턴은 만들지 않는다.
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);

// 시험용 가짜 값이다. 실제 비밀이 아니다.
const SECRET = "synthetic-secret-0000";
const LOG_LINE = "synthetic-log-1111";

/** 가짜 이슈 서비스이다. 원격 글 한 건, 정책 판, 게시 의도 하나를 가진다. */
const issue = {
  remote: { title: "Old title", body: "## Steps\nold steps\n" },
  policyVersion: "p1",
  intent: null,
  /** 다음 게시가 외부 변경을 만나게 한다. */
  externalEdit: false,
  requests: [],
};
const fieldHash = (copy) => ({ version: 1, sha256: createHash("sha256").update(`${copy.title}\n${copy.body}`).digest("hex") });

function preview(text) {
  const unanswered = text.spans.flatMap((span, index) => (span.class === null && span.decision === null ? [index] : []));
  if (unanswered.length > 0) return { status: "needs_confirmation", spanIndexes: unanswered };
  const sharedCopy = { title: text.title, body: text.body };
  const markers = [];
  // 뒤의 구간부터 바꾸면 앞 구간의 위치가 그대로이다.
  const ordered = text.spans.map((span, index) => ({ ...span, index })).sort((left, right) => right.start - left.start);
  for (const span of ordered) {
    const masks = span.class === "log" || span.decision === "restorable_mask";
    const marker = `[[masked:${span.index + 1}]]`;
    if (masks) markers.unshift({ marker, field: span.field });
    const value = sharedCopy[span.field];
    sharedCopy[span.field] = value.slice(0, span.start) + (masks ? marker : "[removed]") + value.slice(span.end);
  }
  const errors = sharedCopy.body.includes("## Impact") ? [] : [{ field: "required_section", label: "Impact", kind: "missing" }];
  return {
    status: "ready",
    sharedCopy,
    markers,
    errors,
    validatorAbsent: false,
    publishable: errors.length === 0,
    approval: { sharedCopyHash: fieldHash(sharedCopy), policyVersion: issue.policyVersion, templateVersion: "t1" },
  };
}

function answerIssue(method, params) {
  issue.requests.push({ method, params });
  if (method === "akra/issue/open") {
    return {
      ...issue.remote,
      editBasis: fieldHash(issue.remote),
      template: { status: "resolved", path: ".github/ISSUE_TEMPLATE/bug.md", reason: null, requiredInputs: [{ label: "Steps", kind: "section", options: [] }], unsupportedInputs: [] },
      requiredSections: ["Impact"],
      spanActions: { secret: "remove", name: "restorable_mask", path: "restorable_mask", log: "restorable_mask" },
      intent: issue.intent,
    };
  }
  if (method === "akra/issue/preview") return preview(params.text);
  if (method === "akra/issue/publish") {
    const current = preview(params.text);
    if (current.status !== "ready" || !current.publishable) return { status: "blocked", preview: current };
    if (params.approval.sharedCopyHash.sha256 !== current.approval.sharedCopyHash.sha256) return { status: "refused", reason: "shared_copy" };
    if (params.approval.policyVersion !== issue.policyVersion) return { status: "refused", reason: "policy_version" };
    if (issue.externalEdit) {
      issue.externalEdit = false;
      issue.remote = { title: "Edited elsewhere", body: "## Steps\nedited by another person\n" };
    }
    if (params.editBasis.sha256 !== fieldHash(issue.remote).sha256) return { status: "conflict" };
    if (current.markers.length > 0 && !params.sharedCopyOnly) return { status: "held", intent: { intentId: "held-1", state: "held" } };
    // 쓰기는 되었지만 답을 잃은 경우이다.
    issue.remote = current.sharedCopy;
    issue.intent = { intentId: "intent-1", state: "needs_confirmation" };
    return { status: "needs_confirmation", intent: issue.intent };
  }
  if (method === "akra/issue/recheck") {
    if (issue.intent) issue.intent = { ...issue.intent, state: "confirmed" };
    return { intent: issue.intent };
  }
  return { intent: { intentId: params.intentId, state: "abandoned" } };
}

const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], { issue: answerIssue });
const userData = await mkdtemp(join(tmpdir(), "akra-issue-draft-ui-"));
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
    for (const channel of ["akra:github-project", "akra:github-repository"]) ipcMain.removeHandler(channel);
    ipcMain.handle("akra:github-project", () => ({
      title: "Sample board",
      url: "https://github.com/users/sample/projects/1",
      items: [{ kind: "issue", title: "Issue 7", url: "https://github.com/sample/widgets/issues/7", repository: "sample/widgets", number: 7, status: "Todo" }],
      closedCount: 0,
      truncated: false,
    }));
    ipcMain.handle("akra:github-repository", (_event, cwd) => (cwd === "/work/example-app" ? "sample/widgets" : null));
  });
  const page = await app.firstWindow();
  const projects = page.getByTestId("github-project-panel");
  /** 스레드를 고르고 프로젝트 패널에서 이슈 7 의 편집기를 연다. 저장한 프로젝트가 없으면 먼저 저장한다. */
  const openEditor = async () => {
    await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
    await page.getByTestId("thread-entry").first().click();
    await page.getByTestId("work-toggle").click();
    await page.getByTestId("projects-toggle").click();
    const edit = projects.getByTestId("github-project-edit").first();
    await projects.getByTestId("github-project-form").or(edit).first().waitFor({ timeout: 10_000 });
    if ((await edit.count()) === 0) {
      await projects.getByLabel("Project").fill("sample/1");
      await projects.getByRole("button", { name: "저장" }).click();
    }
    await edit.click();
  };
  await openEditor();

  const panel = page.getByTestId("issue-draft-panel");
  const title = panel.getByTestId("issue-draft-title");
  const body = panel.getByTestId("issue-draft-body");
  const approve = panel.getByTestId("issue-draft-approve");
  const sharedBody = panel.getByTestId("issue-draft-shared-body");
  const status = panel.getByTestId("issue-draft-status");
  const until = async (condition) => {
    const deadline = Date.now() + 10_000;
    while (Date.now() < deadline) {
      if (await condition()) return true;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    return false;
  };
  const statusIs = (kind) => until(async () => (await status.count()) > 0 && (await status.getAttribute("data-status")) === kind);
  const select = async (text) => {
    await body.evaluate((element, wanted) => {
      const start = element.value.indexOf(wanted);
      element.focus();
      element.setSelectionRange(start, start + wanted.length);
    }, text);
  };

  await title.waitFor({ timeout: 10_000 });
  check("the editor starts with the remote title", await until(async () => (await title.inputValue()) === "Old title"));
  const requirements = await panel.getByTestId("issue-draft-requirements").innerText();
  check("template fields and required sections are listed", requirements.includes('"Steps"') && requirements.includes('"Impact"'));
  check("the open request names the issue and the folder", JSON.stringify(issue.requests[0].params) === JSON.stringify({ cwd: "/work/example-app", owner: "sample", repository: "widgets", number: 7 }));

  await body.fill(`## Steps\nrun with ${SECRET}\n${LOG_LINE}\n`);
  check("a missing required section is a validation error", await until(async () => (await panel.getByTestId("issue-draft-errors").count()) > 0 && (await panel.getByTestId("issue-draft-errors").innerText()).includes('"Impact"')));
  check("an invalid draft cannot be approved", await approve.isDisabled());

  await select(SECRET);
  await panel.getByTestId("issue-draft-mark-body").click();
  check("a secret span is removed from the shared copy", await until(async () => (await sharedBody.innerText()).includes("[removed]") && !(await sharedBody.innerText()).includes(SECRET)));

  await select(LOG_LINE);
  await panel.getByTestId("issue-draft-span-class").selectOption("log");
  await panel.getByTestId("issue-draft-mark-body").click();
  check("a masked span leaves one marker that the preview lists", await until(async () => (await panel.getByTestId("issue-draft-markers").count()) > 0 && (await panel.getByTestId("issue-draft-markers").innerText()).includes("[[masked:2]]")));
  check("the masked text is not in the shared copy", !(await sharedBody.innerText()).includes(LOG_LINE));

  await body.fill(`## Steps\nrun with ${SECRET}\n${LOG_LINE}\n\n## Impact\nlogin\n`);
  check("text added after the spans keeps both spans", await until(async () => (await panel.getByTestId("issue-draft-span").count()) === 2 && (await approve.isEnabled())));
  check("the shared copy still hides both values", !(await sharedBody.innerText()).includes(SECRET) && !(await sharedBody.innerText()).includes(LOG_LINE));

  await approve.check();
  await title.fill("New title");
  check("a content change clears the approval", await until(async () => !(await approve.isChecked()) && (await panel.getByTestId("issue-draft-notice").count()) > 0));
  check("publish is disabled without an approval", await panel.getByTestId("issue-draft-publish").isDisabled());

  await until(async () => (await panel.getByTestId("issue-draft-shared-title").innerText()) === "New title");
  await approve.check();
  issue.policyVersion = "p2";
  await panel.getByTestId("issue-draft-publish").click();
  check("an approval of an older policy is refused", await statusIs("refused"));
  check("the refused approval is cleared", await until(async () => !(await approve.isChecked())));
  check("nothing was written after the refusal", issue.remote.title === "Old title");

  // 거절 뒤의 새 미리보기는 새 정책 판의 승인 값을 가진다.
  await until(async () => issue.requests.filter((request) => request.method === "akra/issue/preview").at(-1)?.method === "akra/issue/preview" && (await approve.isEnabled()));
  await new Promise((resolve) => setTimeout(resolve, 600));
  await approve.check();
  issue.externalEdit = true;
  await panel.getByTestId("issue-draft-publish").click();
  check("an external edit is shown as a known external change", await statusIs("external_change"));
  check("the external change status names its state", (await status.innerText()).includes("알려진 외부 변경"));
  await panel.getByTestId("issue-draft-reload").click();
  check("reload shows the remote text and keeps the local draft", await until(async () => (await panel.getByTestId("issue-draft-remote").count()) > 0) && (await title.inputValue()) === "New title");
  check("reload clears the approval", !(await approve.isChecked()));

  await approve.check();
  await panel.getByTestId("issue-draft-publish").click();
  check("a restorable mask without a saved record is held", await statusIs("held"));
  await panel.getByTestId("issue-draft-shared-only").click();
  check("a lost answer is shown as publish result needs confirmation", await statusIs("needs_confirmation"));
  check("the needs-confirmation status names its state", (await status.innerText()).includes("게시 결과 확인 필요"));
  check("no new publish can start while the result is unknown", await panel.getByTestId("issue-draft-publish").isDisabled());
  const publishes = issue.requests.filter((request) => request.method === "akra/issue/publish").length;

  // #2429: 창을 다시 불러온다. renderer 의 상태는 모두 없어진다. 다시 연 편집기는 `open` 이 준 의도 상태를 보여주고
  // 게시를 다시 보내지 않는다.
  await page.reload();
  await openEditor();
  check("after a reload the editor shows the unknown publish result again", await statusIs("needs_confirmation"));
  check("after a reload no new publish can start", await panel.getByTestId("issue-draft-publish").isDisabled());
  check("the reload sent no publish", issue.requests.filter((request) => request.method === "akra/issue/publish").length === publishes);
  check("the local text did not survive the reload", !(await page.locator("body").innerText()).includes(SECRET) && !(await body.inputValue()).includes(LOG_LINE));

  await panel.getByTestId("issue-draft-recheck").click();
  check("a recheck that sees the approved copy is publish confirmed", await statusIs("confirmed"));
  check("the confirmed status names its state", (await status.innerText()).includes("게시 확인"));
  check("the recheck sent no new publish", issue.requests.filter((request) => request.method === "akra/issue/publish").length === publishes);
  check("after the publish the editor holds the remote shared copy", await until(async () => (await body.inputValue()).includes("[[masked:2]]") && !(await body.inputValue()).includes(SECRET)));

  const stored = await page.evaluate(() => JSON.stringify({ ...window.localStorage }) + JSON.stringify({ ...window.sessionStorage }));
  check("the local text is not in browser storage", !stored.includes(SECRET) && !stored.includes(LOG_LINE));
  const composer = page.getByTestId("composer-input");
  check("the local text is not in the chat composer", (await composer.count()) === 0 || !(await composer.inputValue()).includes(SECRET));
  check("no turn was started", !gateway.receivedMethods.includes("turn/start") && !gateway.receivedMethods.includes("turn/steer"));

  console.log("--- issue draft panel text ---");
  console.log(await panel.innerText());
  console.log("------------------------------");
  if (process.env.AKRA_QA_SCREENSHOT) {
    await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].showInactive());
    await page.screenshot({ path: process.env.AKRA_QA_SCREENSHOT, animations: "disabled" });
  }

  await panel.getByTestId("issue-draft-done").click();
  check("closing returns to the project list", await until(async () => (await page.getByTestId("github-project-panel").count()) > 0));
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
