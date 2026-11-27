// 화면 시험 (#2625): 다른 Codex 앱이 쓰기 잠금을 가진 대화에서 "채팅 포크" 로 새 대화를 만든다. 실제 codex 와 턴은
// 쓰지 않는다 (가짜 Gateway 의 합성 답). userData 는 임시 폴더를 쓴다. AKRA_QA_SCREENSHOT_DIR 이 있으면 화면을 저장한다.
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);

const SOURCE = "01a1abfb-8f0a-7581-a8e4-c2953347f470";
const CHILD = "01a1abfb-0000-7000-8000-00000000f0f0";
// thread/start 녹화의 스레드 모양을 그대로 쓰고 id 와 계보만 바꾼다.
const recordedThread = JSON.parse(JSON.parse(readFileSync(fake.THREAD_START_RECORDING, "utf8").split(/\r?\n/)[1]).text).result.thread;
const thread = (id, forkedFromId = null) => ({ ...recordedThread, id, sessionId: id, name: "Run node --version", forkedFromId, turns: [] });
const turn = (id, text) => ({
  id, itemsView: "full", status: "completed", error: null, startedAt: 1793886755, completedAt: 1793886760, durationMs: 5000,
  items: [
    { type: "userMessage", id: `${id}-user`, clientId: null, content: [{ type: "text", text, text_elements: [] }] },
    { type: "agentMessage", id: `${id}-agent`, text: `답: ${text}`, phase: "final_answer", memoryCitation: null, delivery: null, questions: null },
  ],
});
const turns = [turn("turn-2", "두 번째 질문"), turn("turn-1", "첫 질문")];
const requests = [];
/** true 면 thread/fork 에 새 대화 id 가 없는 답을 준다: 결과 불명이다. */
let malformedFork = false;
let forkCreated = false;

const userData = mkdtempSync(join(tmpdir(), "akra-e2e-fork-"));
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.APPROVAL_TURN_RECORDING], {
  appServer(method, params) {
    if (!method.startsWith("thread/") && !method.startsWith("turn/")) return undefined;
    requests.push({ method, params });
    const id = params.threadId;
    // 실제 app-server 의 목록은 자식에도 forkedFromId: null 을 준다.
    if (method === "thread/list" && forkCreated) return { result: { data: [thread(SOURCE), thread(CHILD)], nextCursor: null } };
    if (method === "thread/resume" && id === SOURCE) return { error: { code: -32603, message: `thread ${SOURCE} already has an active writer` } };
    if (method === "thread/resume" || method === "thread/read") return { result: { thread: id === CHILD ? thread(CHILD, SOURCE) : thread(SOURCE) } };
    if (method === "thread/turns/list") return { result: { data: params.itemsView === "notLoaded" ? turns.map((item) => ({ ...item, items: [] })) : turns, nextCursor: null, backwardsCursor: null } };
    if (method === "thread/fork" && malformedFork) return { result: {} };
    if (method === "thread/fork") forkCreated = true;
    if (method === "thread/fork") return { result: { thread: thread(CHILD, SOURCE), model: "gpt-6-luna", modelProvider: "openai", serviceTier: null, disabledPluginIds: [], cwd: recordedThread.cwd, instructionSources: [], approvalPolicy: "untrusted", approvalsReviewer: "user", sandbox: { type: "readOnly", networkAccess: false }, reasoningEffort: "low" } };
    if (method === "thread/unsubscribe") return { result: { status: "unsubscribed" } };
    return undefined;
  },
});
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;

const failures = [];
const check = (name, ok, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${!ok && detail ? ` | ${detail}` : ""}`);
  if (!ok) failures.push(name);
};
const shotDir = process.env.AKRA_QA_SCREENSHOT_DIR;
const methodsFor = (threadId) => requests.filter((request) => request.params.threadId === threadId).map((request) => request.method);

const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });

  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("writer-elsewhere-banner").waitFor({ timeout: 15_000 });
  const forkButton = page.getByTestId("writer-elsewhere-fork");
  await page.waitForFunction(() => !document.querySelector('[data-testid="writer-elsewhere-fork"]')?.hasAttribute("disabled"), null, { timeout: 10_000 });
  check("the locked thread offers a chat fork next to reconnect", (await forkButton.textContent())?.includes("채팅 포크") === true);
  check("the fork button explains that Git files are not separated", ((await forkButton.getAttribute("title")) ?? "").includes("Git 작업 파일과 브랜치는 나뉘지 않고"));
  check("the top bar does not repeat the fork action on a locked thread", (await page.getByTestId("thread-fork").count()) === 0);
  if (shotDir) await page.screenshot({ path: join(shotDir, "locked-source.png"), animations: "disabled" });

  await forkButton.click();
  await page.getByTestId("forked-from-banner").waitFor({ timeout: 15_000 });
  const fork = requests.find((request) => request.method === "thread/fork");
  check("one thread/fork pins the newest finished turn", requests.filter((request) => request.method === "thread/fork").length === 1
    && fork?.params.lastTurnId === "turn-2" && fork.params.excludeTurns === true && fork.params.ephemeral === false, JSON.stringify(fork?.params));
  check("the locked source gets no resume, turn, or unsubscribe after the fork",
    JSON.stringify(methodsFor(SOURCE).slice(methodsFor(SOURCE).indexOf("thread/fork") - 2)) === JSON.stringify(["thread/read", "thread/turns/list", "thread/fork"]), JSON.stringify(methodsFor(SOURCE)));
  check("the fork is not resumed again", !methodsFor(CHILD).includes("thread/resume"), JSON.stringify(methodsFor(CHILD)));
  check("no turn is started", !requests.some((request) => request.method.startsWith("turn/")));
  check("the read-only banner does not move to the fork", (await page.getByTestId("writer-elsewhere-banner").count()) === 0);
  check("the fork shows the shared-folder warning", ((await page.getByTestId("forked-from-banner").textContent()) ?? "").includes("같은 작업 폴더"));
  check("the sidebar marks the fork once", (await page.getByTestId("fork-badge").count()) === 1);
  check("the fork shows the inherited history", (await page.getByTestId("turn").count()) === 2);
  check("the fork accepts input", !(await page.getByTestId("composer-input").isDisabled()));
  check("a normal thread offers the fork action in the top bar", (await page.getByTestId("thread-fork").textContent())?.includes("대화 분기") === true);
  if (shotDir) await page.screenshot({ path: join(shotDir, "forked-thread.png"), animations: "disabled" });

  await page.getByTestId("forked-from-open").click();
  await page.getByTestId("writer-elsewhere-banner").waitFor({ timeout: 15_000 });
  check("the source opens again from the fork", (await page.getByTestId("forked-from-banner").count()) === 0);

  // renderer 를 다시 시작하면 목록은 계보 없는 자식만 준다. 다시 열 때 서버 메타데이터로 복원해야 한다.
  await page.reload();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.waitForFunction(() => document.querySelectorAll('[data-testid="thread-entry"]').length === 2, null, { timeout: 15_000 });
  check("a restarted session does not infer lineage from the list", (await page.getByTestId("fork-badge").count()) === 0);
  await page.getByTestId("thread-entry").nth(1).click();
  await page.getByTestId("forked-from-banner").waitFor({ timeout: 15_000 });
  check("reopening the fork restores its sidebar badge", (await page.getByTestId("fork-badge").count()) === 1);
  check("reopening the fork restores the origin action", await page.getByTestId("forked-from-open").isVisible());
  check("restoring lineage does not create another fork or start a turn", requests.filter((request) => request.method === "thread/fork").length === 1
    && !requests.some((request) => request.method.startsWith("turn/")));
  await page.getByTestId("forked-from-open").click();
  await page.getByTestId("writer-elsewhere-banner").waitFor({ timeout: 15_000 });

  // 결과 불명 뒤의 두 번째 분기는 renderer 재시작 뒤에도 먼저 묻는다.
  malformedFork = true;
  const forkCount = () => requests.filter((request) => request.method === "thread/fork").length;
  await page.waitForFunction(() => !document.querySelector('[data-testid="writer-elsewhere-fork"]')?.hasAttribute("disabled"), null, { timeout: 10_000 });
  await forkButton.click();
  await page.getByText("분기 결과를 확인하지 못했습니다").first().waitFor({ timeout: 15_000 });
  check("an answer without a new thread id is an unknown result", forkCount() === 2 && (await page.getByTestId("forked-from-banner").count()) === 0);
  await page.waitForFunction(() => !document.querySelector('[data-testid="writer-elsewhere-fork"]')?.hasAttribute("disabled"), null, { timeout: 10_000 });
  await page.reload();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("writer-elsewhere-banner").waitFor({ timeout: 15_000 });
  await page.waitForFunction(() => !document.querySelector('[data-testid="writer-elsewhere-fork"]')?.hasAttribute("disabled"), null, { timeout: 10_000 });
  let questionText = "";
  // confirm 창은 click 을 막는다. 창이 뜨는 즉시 취소한다.
  page.once("dialog", (question) => { questionText = question.message(); void question.dismiss(); });
  await forkButton.click();
  await page.waitForTimeout(500);
  check("a fork after an unknown result and reload asks first", questionText.includes("지난 분기 결과를 확인하지 못했습니다"), questionText);
  check("a cancelled fork sends nothing", forkCount() === 2, String(forkCount()));
} catch (error) {
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  const exited = new Promise((resolve) => app.process().once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  await gateway.close();
  rmSync(userData, { recursive: true, force: true, maxRetries: 5 });
}
process.exit(failures.length === 0 ? 0 : 1);
