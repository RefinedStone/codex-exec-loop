// 합성 Gateway + 실제 Electron. 모델 호출 없이 대화 선택, 확정, 원 task 복구와 CAS 편집을 검증한다.
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
const userData = await mkdtemp(join(tmpdir(), "akra-task-authoring-"));
const requests = [];
const attempts = new Map();
let revision = 7;
let createdCount = 0;
let dropNextCreate = true;
let rejectNextUpdate = true;
const fields = {
  direction_id: "dir-1", title: "Existing completed task", description: "Preserve description",
  acceptance: [{ id: "existing-criterion", scenario: "Old observable result", channel: "screen", essential: true }],
  status: "done", base_priority: 23, dynamic_priority_delta: 2, priority_reason: "keep priority reason",
  depends_on: ["dependency"], blocked_by: ["blocker"],
};
const tasks = [
  { task_id: "task-existing", fields: structuredClone(fields), acceptance_revision: 3, updated_at: "2026-11-25T00:00:00Z" },
  { task_id: "task-active", fields: { ...structuredClone(fields), title: "Running task", status: "in_progress" }, acceptance_revision: 3, updated_at: "2026-11-25T00:00:00Z" },
];
const snapshot = () => ({ type: "list_tasks", available: true, planning_revision: revision, tasks: structuredClone(tasks) });
let gateway;
const kernel = {
  kernel: "authoring-test-repository",
  queries: {
    thread_activity: { type: "thread_activity", threads: [] },
    auto_follow: { type: "auto_follow", max_turns: 0, paused: true },
    planning: { type: "planning", status: "ready_without_task", active_tasks: [], proposed_tasks: [], skipped_tasks: [] },
    parallel: { type: "parallel", enabled: false },
  },
  onRequest(method, params) {
    if (method === "akra/kernel/query" && params.query?.type === "list_tasks") return { result: snapshot() };
    if (method === "akra/kernel/query" && params.query?.type === "history") {
      const records = [...attempts.values()].flatMap((item) => item.records).filter((record) => record.sequence > (params.query.after ?? 0));
      return { result: { type: "history", records, next_after: records.at(-1)?.sequence ?? params.query.after ?? 0 } };
    }
    if (method === "akra/kernel/query" && params.query?.type === "operation") {
      const attempt = [...attempts.values()].find((item) => item.opId === params.query.op_id);
      return { result: attempt ? { type: "operation", op_id: attempt.opId, state: "done", records: attempt.records } : { type: "operation", op_id: params.query.op_id, state: "unknown", records: [] } };
    }
    const command = params.command;
    if (method !== "akra/kernel/command" || !["create_task", "update_task"].includes(command?.type)) return undefined;
    const prior = attempts.get(params.idempotency_key);
    if (prior) {
      assert.deepEqual(command, prior.command, "same logical request keeps the identical command");
      return { result: { processed: true, operation: { op_id: prior.opId, state: "accepted", replayed: true, next: { query: "history", after: prior.opId } } } };
    }
    if (command.type === "update_task" && rejectNextUpdate) {
      rejectNextUpdate = false;
      revision++;
      return { result: { processed: true, rejection: { reason: "planning_revision_stale", message: "다른 곳에서 작업을 수정했습니다. 최신 작업을 다시 읽으세요.", next: [] } } };
    }
    if (command.type === "update_task") assert.equal(command.expected_planning_revision, revision);
    revision++;
    const id = command.type === "create_task" ? "created-task-" + (++createdCount) : command.task_id;
    const task = { task_id: id, fields: structuredClone(command.task), acceptance_revision: 4, updated_at: "2026-11-25T00:01:00Z" };
    const at = tasks.findIndex((item) => item.task_id === id);
    if (at < 0) tasks.push(task); else tasks[at] = task;
    const opId = attempts.size * 2 + 1;
    const records = [
      { sequence: opId, recorded_at: "2026-11-25T00:01:00Z", event: { kind: "command_received", op_id: opId, command: command.type, idempotency_key: params.idempotency_key } },
      { sequence: opId + 1, recorded_at: "2026-11-25T00:01:00Z", event: { kind: "command_completed", op_id: opId, result: "done", task_mutation: { task_id: id, planning_revision: revision } } },
    ];
    attempts.set(params.idempotency_key, { command: structuredClone(command), opId, records });
    const result = { processed: true, result: { type: command.type === "create_task" ? "task_created" : "task_updated", task_id: id, planning_revision: revision } };
    if (command.type === "create_task" && dropNextCreate) {
      dropNextCreate = false;
      setTimeout(() => gateway.disconnectClients(), 20);
      return { result, replyAfter: new Promise(() => {}) };
    }
    return { result };
  },
};
const endpoint = fake.uniqueTestEndpoint();
gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  kernel, onRequest: (method, params) => requests.push({ method, params: structuredClone(params) }),
});
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;
const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  const selectionButton = page.getByTestId("draft-task-from-selection");
  const conversation = page.getByTestId("conversation");
  await conversation.waitFor();
  assert.equal(await selectionButton.count(), 0, "unselected conversation has no task action");
  const before = await conversation.boundingBox();
  const selectedText = await page.locator(".user-message, .agent-message").first().evaluate((element) => {
    const range = document.createRange();
    range.selectNodeContents(element);
    const selection = window.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
    return selection.toString().trim();
  });
  assert.equal(await selectionButton.count(), 0, "selection alone reserves no space");
  assert.deepEqual(await conversation.boundingBox(), before);
  await page.locator(".user-message, .agent-message").first().click({ button: "right" });
  await selectionButton.waitFor();
  await selectionButton.click();
  await page.getByTestId("task-editor").waitFor();
  assert.equal(await page.locator("#task-editor-description").inputValue(), selectedText);
  assert.equal(requests.filter((r) => r.params.command?.type === "create_task").length, 0, "opening a draft does not mutate");
  await page.locator("#task-editor-task-title").fill("User reviewed title");
  await page.getByLabel("완료 기준 추가", { exact: true }).selectOption("screen");
  await page.getByTestId("task-criterion").locator("textarea").fill("The user can see the saved task.");
  await page.getByTestId("task-editor-save").click();
  await page.getByTestId("task-editor-recover").waitFor({ timeout: 30_000 });
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  // The recovery button is explicit; reconnect itself must not replay a mutation.
  assert.equal(requests.filter((r) => r.params.command?.type === "create_task").length, 1);
  await page.getByTestId("task-editor-recover").click();
  await page.getByTestId("task-editor-notice").filter({ hasText: "created-task-1" }).waitFor();
  assert.equal(createdCount, 1, "lost reply recovery does not duplicate a task");
  assert.equal(tasks.at(-1).fields.status, "awaiting_user");
  assert.equal(tasks.at(-1).fields.acceptance[0].channel, "screen");
  assert.equal(await page.getByTestId("task-editor-select").inputValue(), "created-task-1");

  await page.getByTestId("task-editor-select").selectOption("task-existing");
  await page.locator("#task-editor-task-title").fill("Edited existing task");
  await page.getByTestId("task-editor-save").click();
  await page.getByTestId("task-editor-error").filter({ hasText: "최신 작업" }).waitFor();
  await page.getByRole("button", { name: "최신 작업 다시 읽기" }).click();
  await page.locator("#task-editor-task-title").fill("Edited existing task");
  await page.getByTestId("task-editor-save").click();
  await page.getByTestId("task-editor-notice").filter({ hasText: "task-existing" }).waitFor();
  assert.deepEqual(tasks.find((t) => t.task_id === "task-existing").fields, { ...fields, title: "Edited existing task" });
  await page.getByTestId("task-editor-select").selectOption("task-active");
  assert.equal(await page.getByTestId("task-editor-save").isDisabled(), true);
  assert.equal(await page.locator("#task-editor-task-title").isDisabled(), true);
  assert.equal(requests.some((r) => ["turn/start", "turn/steer"].includes(r.method)), false);

  if (process.env.AKRA_TASK_AUTHORING_SCREENSHOT) {
    await page.getByTestId("task-editor-select").selectOption("task-existing");
    const png = await app.evaluate(async ({ BrowserWindow }) =>
      (await BrowserWindow.getAllWindows()[0].webContents.capturePage(undefined, { stayHidden: true })).toPNG().toString("base64"));
    await writeFile(process.env.AKRA_TASK_AUTHORING_SCREENSHOT, Buffer.from(png, "base64"));
  }
  console.log("PASS task authoring: selection, explicit confirmation, reply-loss recovery, CAS, preservation, active lock, no model turns");
} finally {
  await app.close();
  await gateway.close();
  await rm(userData, { recursive: true, force: true });
}
