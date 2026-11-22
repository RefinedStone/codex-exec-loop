// 화면 시험 (plan 18 K3-6): 빌드한 앱을 가짜 Gateway 와 가짜 저장소 커널에 붙여 커널 패널을 확인한다. 실제 codex 와
// 턴은 쓰지 않는다. 패널을 열면 저장소 커널의 상세를 읽고, 닫으면 결정 알림용 구독만 유지한다.
// 창은 숨긴 채로 돌리고 패널의 글을 출력한다 (숨긴 창 텍스트 캡처).
import { createRequire } from "node:module";
import { once } from "node:events";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);

const THREAD_ID = "01a1abfb-8f0a-7581-a8e4-c2953347f470";
const task = (rank, id, title, status) => ({ rank, task_id: id, direction_title: "Desktop", title, status, priority: 10 });
let releaseParallelReply = null;
let parallelCommandReceived = null;
let rejectNextParallel = false;
const waitForParallelCommand = () => new Promise((resolve, reject) => {
  const timeout = setTimeout(() => reject(new Error("the parallel command did not reach the fake kernel")), 10_000);
  parallelCommandReceived = () => { clearTimeout(timeout); parallelCommandReceived = null; resolve(); };
});
const kernel = {
  kernel: "sample-repo-0123abcd",
  queries: {
    thread_activity: { type: "thread_activity", threads: [] },
    auto_follow: { type: "auto_follow", max_turns: 3, paused: false },
    planning: {
      type: "planning",
      status: "ready_with_task",
      queue_summary: "2 tasks",
      next_task: task(1, "task-1", "Show kernel events", "in_progress"),
      active_tasks: [task(1, "task-1", "Show kernel events", "in_progress"), task(2, "task-2", "Write the e2e", "ready")],
      proposed_tasks: [],
      skipped_tasks: [{ task_id: "task-3", title: "Claude backend", status: "blocked", reason: "waits for R7" }],
    },
    parallel: {
      type: "parallel",
      enabled: true,
      supervisor: {
        state: "supervise",
        pool: { configured: 2, idle: 1, leased: 1, running: 1, awaiting_cleanup: 0, blocked: 0, missing: 0, unavailable: 0, exhausted: false },
        agents: [
          {
            agent_id: "agent-1",
            task_title: "Write the e2e",
            slot_id: "slot-1",
            branch: "akra/task-2",
            state: "running",
            duration: "2m",
            summary: "editing tests",
          },
          {
            agent_id: "agent-2",
            thread_id: THREAD_ID,
            task_title: "Show kernel events",
            slot_id: "slot-2",
            branch: "akra/task-1",
            state: "failed",
            duration: "5m",
            summary: "tests failed",
          },
        ],
        merge_head: "head: task-2",
        merge_queue: [{ task_title: "Write the e2e", state: "pr_pending", branch: "akra/task-2", commit: "abc1234" }],
      },
    },
  },
  onRequest(method, params) {
    const type = params.command?.type;
    if (method !== "akra/kernel/command" || !["enable_parallel", "disable_parallel"].includes(type)) return undefined;
    if (rejectNextParallel) {
      rejectNextParallel = false;
      return { result: { processed: false, rejection: { reason: "kernel_unreachable", message: "synthetic control failure", next: [] } } };
    }
    return {
      result: { processed: true },
      replyAfter: new Promise((resolve) => {
        releaseParallelReply = (enabled, status) => {
          kernel.queries.parallel = { type: "parallel", enabled, status };
          releaseParallelReply = null;
          resolve();
        };
        parallelCommandReceived?.();
      }),
    };
  },
  eventsAfterAttach: [
    {
      type: "thread_activity",
      threads: [
        {
          thread_id: THREAD_ID,
          running: true,
          active_turn_id: "turn-1",
          auto_follow_active: false,
          approval: { approval_id: "approval-1", server_request_id: "0", summary: "run tests", details: [] },
        },
      ],
    },
  ],
};

const endpoint = fake.uniqueTestEndpoint();
const requests = [];
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  kernel, onRequest: (method, params) => requests.push({ method, params: structuredClone(params) }),
});
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1" };
delete env.ELECTRON_RUN_AS_NODE;

const failures = [];
const check = (name, ok) => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}`);
  if (!ok) failures.push(name);
};

const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });

  await page.getByTestId("kernel-toggle").click();
  await page.getByTestId("kernel-panel").waitFor({ timeout: 10_000 });
  check("without an open thread the panel does not attach", !gateway.receivedMethods.includes("akra/kernel/attach"));

  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("kernel-identity").waitFor({ timeout: 15_000 });
  check("the panel shows the repository kernel", (await page.getByTestId("kernel-identity").getAttribute("data-kernel")) === kernel.kernel);
  await page.getByTestId("kernel-task").first().waitFor({ timeout: 10_000 });
  check("the planning queue renders", (await page.getByTestId("kernel-task").count()) === 2);
  check("parallel mode and agents render", (await page.getByTestId("kernel-parallel-mode").textContent())?.includes("켜짐") === true);
  check("the parallel agent and merge queue render", (await page.getByTestId("kernel-agent").count()) === 2 && (await page.getByTestId("kernel-merge-item").count()) === 1);
  check("the panel asks the kernel to read the planning queue", gateway.receivedMethods.includes("akra/kernel/command"));

  await page.getByTestId("kernel-thread").waitFor({ timeout: 10_000 });
  const threadRow = (await page.getByTestId("kernel-thread").textContent()) ?? "";
  check("a thread_activity event shows the running thread with its approval", threadRow.includes("승인 대기: run tests"));

  // 워커 상태판 (#2378): 두 워커의 상태와 worktree 를 한 화면에서 구분하고, 실패한 워커에서 실패 카드와 로그로 간다.
  const workers = page.getByTestId("kernel-agent");
  const workerTexts = await workers.allInnerTexts();
  check(
    "two workers show their own status and worktree",
    (await workers.nth(0).getAttribute("data-status")) === "running" &&
      (await workers.nth(1).getAttribute("data-status")) === "failed" &&
      workerTexts[0]?.includes("slot-1 · akra/task-2") === true &&
      workerTexts[1]?.includes("slot-2 · akra/task-1") === true &&
      workerTexts[1]?.includes("tests failed") === true,
  );
  check("a worker without a thread has no link", (await workers.nth(0).getByRole("button").count()) === 0);
  await workers.nth(1).getByRole("button").click();
  const failedCard = page.locator('[data-testid="item-command"][data-status="failed"]');
  await failedCard.waitFor({ timeout: 10_000 });
  check("the failed card starts folded", (await failedCard.locator(".step-output").count()) === 0);
  await failedCard.locator(".step-line").click();
  check("the open failed card shows the stored log", ((await failedCard.locator(".step-output").innerText()) ?? "").startsWith("$ "));
  console.log("--- failed tool card text ---");
  console.log(await failedCard.innerText());

  const parallelControl = page.getByTestId("kernel-parallel-control");
  check("opening the panel never enables the pool", !requests.some(({ method, params }) =>
    method === "akra/kernel/command" && ["enable_parallel", "disable_parallel"].includes(params.command?.type)));
  const stopReceived = waitForParallelCommand();
  await parallelControl.click();
  await stopReceived;
  await parallelControl.filter({ hasText: "중단 요청 중" }).waitFor();
  check("pending stop retains the last confirmed state", (await page.getByTestId("kernel-parallel-mode").innerText()).includes("켜짐")
    && (await page.getByTestId("kernel-parallel-mode").innerText()).includes("마지막 확인"));
  await page.waitForFunction(() => document.querySelector('[data-testid="kernel-parallel-control"]')?.disabled === true);
  if (!releaseParallelReply) throw new Error("the explicit stop command did not reach the fake kernel");
  releaseParallelReply(false, "parallel mode disabled");
  await parallelControl.filter({ hasText: "병렬 실행 시작" }).waitFor();
  check("stop is confirmed by the kernel query", (await page.getByTestId("kernel-parallel-mode").innerText()).includes("꺼짐"));

  rejectNextParallel = true;
  await parallelControl.click();
  await page.getByTestId("kernel-parallel-error").filter({ hasText: "synthetic control failure" }).waitFor();
  check("failed start shows uncertainty without enabling the pool", await parallelControl.isDisabled()
    && (await page.getByTestId("kernel-parallel-mode").innerText()).includes("확인 필요")
    && (await page.getByTestId("kernel-parallel-mode").innerText()).includes("꺼짐"));
  await page.getByTestId("kernel-panel").getByRole("button", { name: "새로 고침" }).click();
  await page.waitForFunction(() => document.querySelector('[data-testid="kernel-parallel-control"]')?.disabled === false);
  const startReceived = waitForParallelCommand();
  await parallelControl.click();
  await startReceived;
  await parallelControl.filter({ hasText: "시작 요청 중" }).waitFor();
  if (!releaseParallelReply) throw new Error("the explicit start command did not reach the fake kernel");
  releaseParallelReply(false, "parallel readiness blocked: public repository requires parent opt-in");
  await parallelControl.filter({ hasText: "병렬 실행 시작" }).waitFor();
  await page.getByTestId("kernel-parallel-status").filter({ hasText: "public repository requires parent opt-in" }).waitFor();
  check("accepted start does not hide a readiness failure", (await page.getByTestId("kernel-parallel-mode").innerText()).includes("꺼짐")
    && (await page.getByTestId("kernel-parallel-notice").innerText()).includes("접수"));
  kernel.queries.parallel = { type: "parallel", enabled: true, status: "parallel ready" };
  gateway.sendKernelEvent(kernel.queries.parallel);
  await parallelControl.filter({ hasText: "병렬 실행 중단" }).waitFor();
  check("the applied parallel state follows kernel events", (await page.getByTestId("kernel-parallel-mode").innerText()).includes("켜짐"));
  check("parallel commands never send model requests from the renderer", !requests.some(({ method }) =>
    method === "turn/start" || method === "turn/steer"));

  console.log("--- kernel panel text ---");
  console.log(await page.getByTestId("kernel-panel").innerText());
  console.log("-------------------------");

  const beforeClose = requests.length;
  await page.getByTestId("kernel-toggle").click();
  await page.getByTestId("kernel-panel").waitFor({ state: "detached", timeout: 10_000 });
  check("closing the kernel panel removes its view", (await page.getByTestId("kernel-panel").count()) === 0);

  // 실제 구독으로 resync를 받은 뒤 같은 연결의 응답까지 기다려, 단순 지연 없이 후속 요청을 확인한다.
  await page.evaluate(() => {
    globalThis.kernelPanelClosedResync = new Promise((resolve, reject) => {
      const id = "kernel-panel-closed-resync";
      const timer = setTimeout(() => {
        unsubscribe();
        reject(new Error("the closed kernel subscription did not complete its resync barrier"));
      }, 5_000);
      const unsubscribe = window.akra.onMessage((text) => {
        const message = JSON.parse(text);
        if (message.method === "akra/kernel/event" && message.params?.event?.type === "resync") {
          window.akra.send(JSON.stringify({ jsonrpc: "2.0", id, method: "akra/gateway/info" }));
        } else if (message.id === id) {
          clearTimeout(timer);
          unsubscribe();
          if (message.error) reject(new Error("the closed kernel resync barrier failed"));
          else resolve(true);
        }
      });
    });
  });
  gateway.sendKernelEvent({ type: "resync" });
  check("the closed panel retains its live metadata-only kernel subscription", await page.evaluate(() => globalThis.kernelPanelClosedResync));
  await page.evaluate(() => { delete globalThis.kernelPanelClosedResync; });

  const closedRequests = requests.slice(beforeClose);
  check("closing the panel keeps the existing kernel attachment", !closedRequests.some(({ method }) => method === "akra/kernel/detach" || method === "akra/kernel/attach"));
  check("the closed panel stops overview and planning reads after resync", !closedRequests.some(({ method, params }) =>
    (method === "akra/kernel/query" && ["thread_activity", "auto_follow", "planning", "parallel"].includes(params.query?.type)) ||
    (method === "akra/kernel/command" && params.command?.type === "refresh_planning")));
  check("the closed panel does not read issue content after resync", !closedRequests.some(({ method, params }) =>
    method === "akra/kernel/query" && ["issues", "issue", "issue_automation", "issue_automation_policy"].includes(params.query?.type)));
} catch (error) {
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  // 창을 닫으면 앱이 끝난다. app.close() 는 쓰지 않는다 (P2 기록 참고).
  const exited = once(app.process(), "exit", { signal: AbortSignal.timeout(10_000) });
  for (const window of app.windows()) await window.close();
  await exited;
  await gateway.close();
}
process.exit(failures.length === 0 ? 0 : 1);
