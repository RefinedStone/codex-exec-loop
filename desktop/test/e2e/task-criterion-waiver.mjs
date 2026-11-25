// #2721: real Electron IPC/utility process with a synthetic Gateway/helper/native dialog.
// This is not an OS credential or production restore verification.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { _electron as electron } from "playwright-core";
import { confirmationReply, confirmedRequest, createConfirmationFixture } from "../fixtures/confirmationBridge.mjs";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);
const { taskEvidenceFixture } = await import("../fixtures/taskEvidence.ts");
const snapshot = taskEvidenceFixture();
const task = snapshot.tasks[0];
const entry = task.criteria[0];
const requests = [];
const summaryTask = { rank: 1, task_id: task.scope.task_id, direction_title: "Fixture", title: "Waiver fixture", status: "in_progress", priority: 1 };
const kernel = {
  kernel: "waiver-fixture",
  queries: {
    thread_activity: { type: "thread_activity", threads: [] },
    auto_follow: { type: "auto_follow", max_turns: 0, paused: false },
    parallel: { type: "parallel", enabled: false },
    planning: { type: "planning", status: "ready_with_task", revision: snapshot.planning_revision,
      next_task: summaryTask, active_tasks: [summaryTask], proposed_tasks: [], skipped_tasks: [], task_evidence: snapshot },
  },
  onRequest(method, params) {
    const prepared = confirmationReply(method, params);
    if (prepared) return prepared;
    if (method === "akra/kernel/command_confirmed") {
      const request = confirmedRequest(params);
      const command = request.command;
      assert.equal(command.type, "waive_criterion");
      if (command.scope_id !== task.scope.scope_id || command.expected_acceptance_revision !== task.scope.acceptance_revision) {
        return { result: { processed: false, rejection: { reason: "conflict", message: "scope changed", next: [] } } };
      }
      const waiver = { scope: structuredClone(task.scope), criterion_id: entry.criterion.id,
        reason_digest: createHash("sha256").update(command.reason.trim()).digest("hex"), by: "human", recorded_at: "2026-11-25T00:00:00Z" };
      snapshot.evidence_revision++;
      const receipt = { event_id: "criterion-waiver:" + command.request_id, evidence_revision: snapshot.evidence_revision, waiver };
      entry.state = "waived";
      entry.waiver = receipt;
      return { result: { processed: true, result: { type: "criterion_waived", changed: true, receipt } } };
    }
    if (method === "akra/kernel/command" && params.command?.type === "refresh_planning") return { result: { processed: true } };
    return undefined;
  },
};
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  kernel, onRequest: (method, params) => requests.push({ method, params: structuredClone(params) }),
});
const userData = await mkdtemp(join(tmpdir(), "akra-criterion-waiver-ui-"));
const fixture = await createConfirmationFixture(userData);
const env = { ...process.env, ...fixture.environment, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;
let app;
let appProcess;
const submitted = () => requests.filter((request) => request.method === "akra/kernel/command_confirmed");
const until = async (predicate) => {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    if (await predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error("waiver fixture did not reach its expected state");
};
try {
  app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env, cwd: fixture.cwd });
  appProcess = app.process();
  await app.evaluate(({ dialog }) => {
    globalThis.waiverDialogs = [];
    dialog.showMessageBox = (_parent, options) => new Promise((resolve) => {
      globalThis.waiverDialogs.push({ defaultId: options.defaultId, cancelId: options.cancelId });
      globalThis.answerWaiver = (response) => resolve({ response, checkboxChecked: false });
      options.signal.addEventListener("abort", () => resolve({ response: 1, checkboxChecked: false }), { once: true });
    });
  });
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("kernel-toggle").click();
  const evidence = page.locator('[data-testid="task-evidence"][data-task-id="task-1"]');
  await evidence.locator("summary").first().click();
  let form = evidence.getByTestId("task-criterion-waiver");
  await form.locator("summary").first().click();
  assert.equal(await form.getByTestId("task-waiver-confirm").isDisabled(), true);
  await form.getByTestId("task-waiver-reason").fill("  ");
  assert.equal(await form.getByTestId("task-waiver-confirm").isDisabled(), true);
  await form.getByTestId("task-waiver-reason").fill("첫 면제 사유");
  await form.getByTestId("task-waiver-confirm").click();
  await until(() => app.evaluate(() => globalThis.waiverDialogs.length === 1));
  assert.deepEqual(await app.evaluate(() => globalThis.waiverDialogs[0]), { defaultId: 1, cancelId: 1 });
  assert.equal(submitted().length, 0);
  await app.evaluate(() => globalThis.answerWaiver(1));
  await form.getByTestId("task-waiver-notice").filter({ hasText: "취소했습니다" }).waitFor();
  assert.equal(submitted().length, 0);
  assert.equal(await form.getByTestId("task-waiver-reason").inputValue(), "첫 면제 사유");
  console.log("PASS blank reasons are disabled and native cancellation preserves the draft without submitting proof");

  await page.waitForFunction(() => document.querySelector('[data-testid="task-waiver-confirm"]')?.disabled === false);
  await form.getByTestId("task-waiver-confirm").click();
  await until(() => app.evaluate(() => globalThis.waiverDialogs.length === 2));
  task.scope = { ...task.scope, scope_id: "9".repeat(64), acceptance_revision: 4, criteria_sha256: "8".repeat(64) };
  entry.state = "stale";
  snapshot.planning_revision++;
  kernel.queries.planning.revision = snapshot.planning_revision;
  gateway.sendKernelEvent(kernel.queries.planning);
  await evidence.getByTestId("task-evidence-status").filter({ hasText: "이전 범위의 보고" }).waitFor();
  form = evidence.getByTestId("task-criterion-waiver");
  await form.locator("summary").first().click();
  assert.equal(await form.getByTestId("task-waiver-reason").inputValue(), "");
  await app.evaluate(() => globalThis.answerWaiver(0));
  await until(() => submitted().length === 1);
  assert.equal(entry.state, "stale");
  assert.equal(await evidence.getByTestId("task-evidence-status").getAttribute("data-state"), "stale");
  console.log("PASS a changed scope discards the old draft and the server rejects its previously prepared confirmation");

  await form.getByTestId("task-waiver-reason").fill("현재 범위의 면제 사유");
  await form.getByTestId("task-waiver-confirm").click();
  await until(() => app.evaluate(() => globalThis.waiverDialogs.length === 3));
  await app.evaluate(() => globalThis.answerWaiver(0));
  await evidence.getByTestId("task-evidence-status").filter({ hasText: /^사람 면제$/ }).waitFor();
  assert.equal(submitted().length, 2);
  const command = submitted()[1].params.request.command;
  assert.equal(command.scope_id, task.scope.scope_id);
  assert.equal(command.expected_acceptance_revision, 4);
  assert.equal(command.criterion_id, "criterion with spaces");
  assert.equal(command.reason, "현재 범위의 면제 사유");
  assert.equal(await form.getByTestId("task-waiver-reason").inputValue(), "");
  assert.equal(await form.getByTestId("task-waiver-confirm").isDisabled(), true);
  assert.equal(requests.some((request) => request.method === "akra/kernel/command" && request.params.command?.type === "waive_criterion"), false);
  assert.equal(gateway.receivedMethods.some((method) => ["turn/start", "turn/steer"].includes(method)), false);
  const text = await page.locator("body").innerText();
  assert.equal(text.includes("fixture-private-"), false);
  assert.equal(text.includes("private-session"), false);
  assert.equal(text.includes("현재 범위의 면제 사유"), false);
  console.log("PASS the current exact scope is waived through native proof and requery; renderer displays no proof, raw reason or invented host pass");
} finally {
  if (app && appProcess && appProcess.exitCode === null) {
    const exited = new Promise((resolve) => appProcess.once("exit", resolve));
    for (const window of app.windows()) await window.close().catch(() => {});
    let timer;
    const closed = await Promise.race([exited.then(() => true), new Promise((resolve) => { timer = setTimeout(() => resolve(false), 10_000); })]);
    clearTimeout(timer);
    if (!closed) appProcess.kill();
  }
  await gateway.close();
  await rm(userData, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 });
}
