// 화면 시험 (#2836): 명령·파일 승인 밖의 서버 요청 세 가지가 입력창 위 카드로 보이고, 카드에서 고른 답이 서버에 간다.
// 녹화의 명령 승인 요청을 합성 요청으로 바꿔 재생한다. 실제 codex 와 턴은 쓰지 않는다 (가짜 Gateway).
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);

const failures = [];
const check = (name, ok, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${!ok && detail ? ` | ${detail}` : ""}`);
  if (!ok) failures.push(name);
};

function recordingWith(folder, method, params) {
  const lines = readFileSync(fake.APPROVAL_TURN_RECORDING, "utf8").split(/\r?\n/).filter(Boolean).map((line) => {
    const entry = JSON.parse(line);
    const message = JSON.parse(entry.text);
    if (entry.direction === "gateway" && message.method === "item/commandExecution/requestApproval") {
      const { threadId, turnId, itemId } = message.params;
      entry.text = JSON.stringify({ method, id: message.id, params: { threadId, turnId, itemId, ...params } });
    }
    return JSON.stringify(entry);
  });
  const path = join(folder, `${method.replaceAll("/", "-")}.jsonl`);
  writeFileSync(path, lines.join("\n"));
  return path;
}

const cases = [
  {
    name: "permissions",
    method: "item/permissions/requestApproval",
    params: { environmentId: "local", startedAtMs: 0, cwd: "/work/example-app", reason: "needs network", permissions: { network: { enabled: true }, fileSystem: null } },
    answer: async (card) => {
      check("the permissions card lists the requested network access", (await card.getByTestId("approval-permissions").textContent())?.includes("네트워크") ?? false);
      await card.getByTestId("approval-acceptForSession").click();
    },
    expected: { permissions: { network: { enabled: true } }, scope: "session" },
  },
  {
    name: "user input",
    method: "item/tool/requestUserInput",
    params: { isBlocking: true, autoResolutionMs: null, questions: [{ id: "q1", header: "Branch", question: "Which branch?", isOther: true, isSecret: false, options: [{ label: "main", description: "default" }] }] },
    answer: async (card) => {
      await card.getByLabel("main").check();
      await card.getByTestId("user-input-submit").click();
    },
    expected: { answers: { q1: { answers: ["main"] } } },
  },
  {
    name: "MCP elicitation",
    method: "mcpServer/elicitation/request",
    params: { serverName: "notion", mode: "form", _meta: null, message: "Pick a page", requestedSchema: { type: "object", properties: { page: { type: "string", title: "Page", enum: ["a", "b"] }, count: { type: "integer", title: "Count" } }, required: ["page"] } },
    answer: async (card) => {
      await card.locator("label", { hasText: "Page" }).locator("select").selectOption("b");
      await card.locator("label", { hasText: "Count" }).locator("input").fill("3");
      await card.getByTestId("elicitation-accept").click();
    },
    expected: { action: "accept", content: { page: "b", count: 3 }, _meta: null },
  },
];

const folder = mkdtempSync(join(tmpdir(), "akra-e2e-server-requests-"));
try {
  for (const testCase of cases) {
    const userData = mkdtempSync(join(folder, "user-data-"));
    const endpoint = fake.uniqueTestEndpoint();
    const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, recordingWith(folder, testCase.method, testCase.params)]);
    const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
    delete env.ELECTRON_RUN_AS_NODE;
    const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
    try {
      const page = await app.firstWindow();
      await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
      await page.getByTestId("thread-entry").first().click();
      const card = page.getByTestId("approval");
      await card.waitFor({ timeout: 15_000 });
      check(`the ${testCase.name} request shows a card above the composer`, (await page.getByTestId("approvals").count()) === 1);
      await testCase.answer(card);
      await card.waitFor({ state: "detached", timeout: 5_000 });
      const deadline = Date.now() + 5_000;
      while (gateway.answeredRequests.length === 0 && Date.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 50));
      const result = gateway.answeredRequests[0]?.result;
      check(`the ${testCase.name} answer reaches the server`, JSON.stringify(result) === JSON.stringify(testCase.expected), JSON.stringify(result));
    } catch (error) {
      check(`the ${testCase.name} flow finished without error (${String(error).split("\n")[0]})`, false);
    } finally {
      const exited = new Promise((resolve) => app.process().once("exit", resolve));
      for (const window of app.windows()) await window.close();
      await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
      await gateway.close();
    }
  }
} finally {
  rmSync(folder, { recursive: true, force: true, maxRetries: 5 });
}
process.exit(failures.length === 0 ? 0 : 1);
