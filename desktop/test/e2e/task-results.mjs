// Stored task result reads in actual Electron against a synthetic Gateway; no model turns.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);
const userData = await mkdtemp(join(tmpdir(), "akra-task-results-"));
const requests = [];
let failResult = false;
let failTasks = false;
let lineageComplete = true;
let activeWorkspace = "/work/example-app";
let resultBarrier;
async function until(predicate, label) {
  for (let index = 0; index < 200; index++) {
    if (await predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error("timed out: " + label);
}
const criterion = { id: "criterion", scenario: "The expected output is present", channel: "command", essential: true, verification_recipe: "git_diff_check_v1" };
const tasks = [
  { task_id: "completed-task", fields: { title: "Identical title", status: "done", description: "Original result", acceptance: [criterion] }, acceptance_revision: 3, updated_at: "2026-11-25T00:00:00Z" },
  { task_id: "cancelled-task", fields: { title: "Identical title", status: "cancelled", acceptance: [criterion] }, acceptance_revision: 3, updated_at: "2026-11-25T00:00:00Z" },
  { task_id: "supplemental-task", fields: { title: "Supplemental repair", status: "ready", acceptance: [criterion] }, acceptance_revision: 3, updated_at: "2026-11-25T00:00:00Z" },
  { task_id: "without-pr", fields: { title: "No linked PR yet", status: "ready", acceptance: [] }, acceptance_revision: 1, updated_at: "2026-11-25T00:00:00Z" },
];
function pr(number, source, relation = "source") {
  return { record_id: "validation-" + number, source_task_id: source, work_order_id: "order-1", relation,
    number, url: "https://github.com/synthetic/repository/pull/" + number, state: "observing", phase: number === 41 ? "post_merge_observation" : "pre_merge_observation",
    reason: "Waiting for independent validation", blocker: "required check pending", next_action: "observe required check",
    current: true, successor_record_id: null, merge_sha: number === 41 ? "c".repeat(40) : null,
    integration_method: null, integration_evidence_sha: null };
}
function resultFor(id) {
  const task = tasks.find((task) => task.task_id === id);
  const scope = { task_id: id, acceptance_revision: task?.acceptance_revision ?? 1, scope_id: "a".repeat(64), criteria_sha256: "b".repeat(64) };
  const prs = activeWorkspace === "/work/task-results-other" ? [pr(99, id)]
    : id === "completed-task" ? [pr(41, id), pr(42, id)]
    : id === "cancelled-task" ? [pr(43, id)]
      : id === "supplemental-task" ? [pr(41, "completed-task", "remediation"), pr(44, id)] : [];
  return { type: "task_result", task_id: id, status: task ? "available" : "missing", planning_revision: 7, task: task ?? null,
    runtime_available: true, lineage_available: true, pr_lineage_complete: lineageComplete,
    work_order: id === "without-pr" ? null : { work_order_id: "order-1", related_task_ids: tasks.filter((task) => task.task_id !== id && task.task_id !== "without-pr").map((task) => task.task_id),
      external_reference_url: "https://github.com/synthetic/repository/issues/5", close_on_merge: true },
    sessions: id === "completed-task" ? [{ session_key: "execution-1", agent_id: "agent-1", thread_id: null, branch: "renamed-after-completion",
      state: "finished", completion_state: "completed", worker_validation_summary: "Worker reports pass", delivery_outcome: "queued", updated_at: "2026-11-25T00:01:00Z" }] : [],
    dispatch_blocks: [], deliveries: [], pull_requests: prs,
    evidence: { planning_revision: 7, evidence_revision: 2, unavailable: null, truncated: false, tasks: task ? [{ scope, unavailable: null, source: null,
      criteria: (task.fields.acceptance ?? []).map((criterion) => ({ criterion, state: "pass", evidence: null, waiver: null,
        host_verification: { phase: "finished", state: "failed", unavailable: null, attempt: null, receipt: null } })) }] : [] } };
}
const kernel = {
  kernel: "task-result-test-repository",
  queries: {
    thread_activity: { type: "thread_activity", threads: [] },
    auto_follow: { type: "auto_follow", max_turns: 0, paused: true },
    planning: { type: "planning", status: "ready_without_task", active_tasks: [], proposed_tasks: [], skipped_tasks: [] },
    parallel: { type: "parallel", enabled: false },
  },
  onRequest(method, params) {
    if (method === "akra/kernel/attach") {
      activeWorkspace = params.workspace;
      return { result: { client_id: "1", protocol_version: 1, kernel: activeWorkspace === "/work/task-results-other" ? "task-result-other-repository" : kernel.kernel } };
    }
    if (method !== "akra/kernel/query") return undefined;
    if (params.query?.type === "list_tasks" && failTasks) return { error: { code: -32013, message: "합성 작업 목록 조회 실패" } };
    if (params.query?.type === "list_tasks") return { result: { type: "list_tasks", available: true, planning_revision: 7, tasks } };
    if (params.query?.type === "task_result") {
      if (failResult) return { error: { code: -32013, message: "합성 결과 조회 실패" } };
      return { result: resultFor(params.query.task_id), replyAfter: resultBarrier };
    }
  },
};
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  kernel, onRequest: (method, params) => requests.push({ method, params: structuredClone(params) }),
});
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;
const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  await app.evaluate(({ shell }) => {
    globalThis.taskResultOpened = [];
    shell.openExternal = async (url) => { globalThis.taskResultOpened.push(url); };
  });
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("kernel-toggle").click();
  await page.getByTestId("kernel-panel").waitFor();
  await page.getByRole("button", { name: "작업 편집", exact: true }).click();
  await page.getByTestId("task-editor-select").selectOption("completed-task");
  assert.equal(requests.some((request) => request.params.query?.type === "task_result"), false, "results are only read when opened");
  const commandsBeforeResults = requests.filter((request) => request.method === "akra/kernel/command");
  const view = page.getByTestId("task-result");
  await view.locator(":scope > summary").click();
  await page.getByTestId("task-result-pr").filter({ hasText: "PR #42" }).waitFor();
  assert.equal(await page.getByTestId("task-result-pr").count(), 2);
  const facts = await page.getByTestId("task-result-facts").innerText();
  assert(facts.includes("완료"));
  assert(facts.includes("통과 0개 · 실패 1개 / 전체 기준 1개"), "worker pass cannot become host pass");
  assert(facts.includes("통과 보고 1개"));
  assert(facts.includes("1개에 머지 SHA가 기록됨"));
  assert(facts.includes("외부 이슈 종료\n확인되지 않음"));
  assert.equal(await page.getByTestId("task-result-pr").filter({ hasText: "PR #43" }).count(), 0);
  await page.getByRole("button", { name: "PR #42 열기", exact: true }).click();
  assert.deepEqual(await app.evaluate(() => globalThis.taskResultOpened), ["https://github.com/synthetic/repository/pull/42"]);

  await page.getByTestId("task-editor-select").selectOption("cancelled-task");
  await page.getByTestId("task-result-pr").filter({ hasText: "PR #43" }).waitFor();
  assert.equal(await page.getByTestId("task-result-pr").count(), 1);
  assert((await page.getByTestId("task-result-facts").innerText()).includes("취소"));
  assert.equal(await page.getByTestId("task-result-pr").filter({ hasText: "PR #41" }).count(), 0);

  await page.locator("#task-editor-lookup").fill("supplemental-task");
  await page.getByRole("button", { name: "ID로 열기", exact: true }).click();
  await page.getByTestId("task-result-pr").filter({ hasText: "PR #44" }).waitFor();
  assert.equal(await page.getByTestId("task-result-pr").count(), 2);
  assert((await page.getByTestId("task-result-pr").filter({ hasText: "PR #41" }).innerText()).includes("이 작업이 보완하는 PR"));
  assert((await page.getByTestId("task-result-facts").innerText()).includes("0개에 머지 SHA가 기록됨"), "a parent's merge is not this supplemental task's own PR merge");

  // A failed read of another task keeps the edited task's ID, fields and revision. Both entry paths.
  const title = page.locator("#task-editor-task-title");
  const identity = page.getByTestId("task-editor-identity");
  const editorError = page.getByTestId("task-editor-error").filter({ hasText: "합성 작업 목록 조회 실패" });
  const commandsBeforeFailedReads = requests.filter((request) => request.method === "akra/kernel/command").length;
  await title.fill("Edited supplemental title");
  failTasks = true;
  await page.locator("#task-editor-lookup").fill("completed-task");
  await page.getByRole("button", { name: "ID로 열기", exact: true }).click();
  await editorError.waitFor();
  assert.match(await identity.innerText(), /^supplemental-task /, "a failed ID read keeps the edited task");
  assert.equal(await title.inputValue(), "Edited supplemental title");
  failTasks = false;
  await page.getByRole("button", { name: "ID로 열기", exact: true }).click();
  await identity.filter({ hasText: /^completed-task / }).waitFor();
  assert.equal(await title.inputValue(), "Identical title");
  assert.equal(await editorError.count(), 0);

  await title.fill("Edited completed title");
  failTasks = true;
  await page.getByRole("button", { name: "supplemental-task 열기", exact: true }).click();
  await editorError.waitFor();
  assert.match(await identity.innerText(), /^completed-task /, "a failed linked-task read keeps the edited task");
  assert.equal(await title.inputValue(), "Edited completed title");
  failTasks = false;
  await page.getByRole("button", { name: "supplemental-task 열기", exact: true }).click();
  await identity.filter({ hasText: /^supplemental-task / }).waitFor();
  assert.equal(await title.inputValue(), "Supplemental repair");
  assert.equal(requests.filter((request) => request.method === "akra/kernel/command").length, commandsBeforeFailedReads, "failed reads send no save");
  await page.getByTestId("task-result-pr").filter({ hasText: "PR #44" }).waitFor();

  await page.getByRole("button", { name: "completed-task 열기", exact: true }).click();
  await page.getByTestId("task-result-pr").filter({ hasText: "PR #42" }).waitFor();
  failResult = true;
  await page.getByTestId("task-result-refresh").click();
  await view.getByRole("alert").filter({ hasText: "합성 결과 조회 실패" }).waitFor();
  assert.equal(await page.getByTestId("task-result-facts").getAttribute("data-current"), "false");
  assert.match(await page.getByTestId("task-result-pr").first().innerText(), /마지막 조회의 관찰/);
  assert((await page.getByTestId("task-result-facts").innerText()).includes("현재 상태 미확인"));
  failResult = false;
  lineageComplete = false;
  await page.getByTestId("task-result-refresh").click();
  await page.getByTestId("task-result-lineage-unknown").waitFor();
  assert.equal(await page.getByTestId("task-result-pr").count(), 2);
  assert.equal(await page.getByTestId("task-result-no-pr").count(), 0);

  lineageComplete = true;
  await page.getByTestId("task-editor-select").selectOption("without-pr");
  await page.getByTestId("task-result-no-pr").waitFor();
  assert.equal(await page.getByTestId("task-result-pr").count(), 0);
  await page.locator("#task-editor-lookup").fill("removed-task");
  await page.getByRole("button", { name: "ID로 열기", exact: true }).click();
  await view.getByText("현재 작업 원본에서 이 ID를 찾지 못했습니다.", { exact: true }).waitFor();
  assert.equal(await page.getByTestId("task-editor-save").isDisabled(), true);
  assert.equal(await page.getByTestId("task-result-pr").count(), 0);
  assert.deepEqual(requests.filter((request) => request.method === "akra/kernel/command"), commandsBeforeResults, "result interactions add no command after the existing kernel-panel refresh");
  assert.equal(requests.some((request) => ["turn/start", "turn/steer"].includes(request.method)), false);

  if (process.env.AKRA_TASK_RESULTS_SCREENSHOT) {
    await page.locator("#task-editor-lookup").fill("completed-task");
    await page.getByRole("button", { name: "ID로 열기", exact: true }).click();
    await page.getByTestId("task-result-pr").filter({ hasText: "PR #42" }).waitFor();
    const capture = () => app.evaluate(async ({ BrowserWindow }) =>
      (await BrowserWindow.getAllWindows()[0].webContents.capturePage(undefined, { stayHidden: true })).toPNG().toString("base64"));
    await view.evaluate((element) => element.scrollIntoView({ block: "start" }));
    await capture();
    await writeFile(process.env.AKRA_TASK_RESULTS_SCREENSHOT, Buffer.from(await capture(), "base64"));
    await page.getByTestId("task-result-pr").first().evaluate((element) => element.scrollIntoView({ block: "start" }));
    await capture();
    await writeFile(process.env.AKRA_TASK_RESULTS_SCREENSHOT.replace(/\.png$/, "-pr.png"), Buffer.from(await capture(), "base64"));
  }

  // Hold an old response while changing repositories. The new repository intentionally reuses the ID.
  await page.locator("#task-editor-lookup").fill("completed-task");
  await page.getByRole("button", { name: "ID로 열기", exact: true }).click();
  await page.getByTestId("task-result-pr").filter({ hasText: "PR #42" }).waitFor();
  let releaseOld;
  resultBarrier = new Promise((resolve) => { releaseOld = resolve; });
  const resultsBefore = requests.filter((request) => request.params.query?.type === "task_result").length;
  await page.getByTestId("task-result-refresh").click();
  await until(() => requests.filter((request) => request.params.query?.type === "task_result").length > resultsBefore, "the held original query");
  await page.getByRole("button", { name: "작업 편집 닫기", exact: true }).click();
  resultBarrier = undefined;
  await app.evaluate(({ ipcMain }) => {
    ipcMain.removeHandler("akra:choose-folder");
    ipcMain.handle("akra:choose-folder", () => "/work/task-results-other");
  });
  await page.getByTestId("new-folder-thread").click();
  await page.getByTestId("home").locator(".home-pickers button").first().click();
  await page.getByRole("option", { name: "폴더 고르기…", exact: true }).click();
  await until(async () => (await page.getByTestId("kernel-identity").getAttribute("data-kernel")) === "task-result-other-repository", "the other repository kernel");
  await page.getByRole("button", { name: "작업 편집", exact: true }).click();
  await page.getByTestId("task-editor-select").selectOption("completed-task");
  await page.getByTestId("task-result").locator(":scope > summary").click();
  await page.getByTestId("task-result-pr").filter({ hasText: "PR #99" }).waitFor();
  releaseOld();
  await page.getByTestId("task-result-refresh").click();
  await until(async () => (await page.getByTestId("task-result-facts").getAttribute("data-current")) === "true", "the new repository refresh");
  assert.equal(await page.getByTestId("task-result-pr").count(), 1);
  assert.equal(await page.getByTestId("task-result-pr").filter({ hasText: "PR #42" }).count(), 0);
  assert((await page.getByTestId("task-result-pr").innerText()).includes("PR #99"));
  assert.equal(requests.some((request) => ["turn/start", "turn/steer"].includes(request.method)), false);
  assert.equal(requests.some((request) => request.method === "akra/kernel/command" && request.params.command?.type !== "refresh_planning"), false, "only the existing kernel-panel refresh is allowed");

  console.log("PASS task results: exact completed/cancelled IDs, multiple PRs, supplemental lineage, separate host/worker/merge/closure facts, unavailable/stale and cross-workspace late reads, no result mutations or model turns");
} finally {
  await app.close();
  await gateway.close();
  await rm(userData, { recursive: true, force: true });
}
