// 서버 요청 화면 회귀. 합성 녹화와 가짜 Gateway만 사용하며 실제 codex/모델은 호출하지 않는다.
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

function recordingWith(folder, method, params, additionalRequests = []) {
  let requestId;
  const lines = readFileSync(fake.APPROVAL_TURN_RECORDING, "utf8").split(/\r?\n/).filter(Boolean).flatMap((line) => {
    const entry = JSON.parse(line);
    const message = JSON.parse(entry.text);
    if (entry.direction === "gateway" && message.method === "item/commandExecution/requestApproval") {
      const { threadId, turnId, itemId } = message.params;
      requestId = String(message.id);
      const requests = [{ method, id: message.id, params }, ...additionalRequests];
      return requests.map((request) => JSON.stringify({
        ...entry, text: JSON.stringify({ ...request, params: { threadId, turnId, itemId, ...request.params } }),
      }));
    }
    return [JSON.stringify(entry)];
  });
  const path = join(folder, `${method.replaceAll("/", "-")}.jsonl`);
  writeFileSync(path, lines.join("\n"));
  return { path, requestId };
}

const permissions = {
  network: { enabled: true },
  fileSystem: {
    read: null, write: null, entries: [
      { path: { type: "path", path: "/workspace/source" }, access: "read" },
      { path: { type: "glob_pattern", pattern: "/workspace/build/**" }, access: "write" },
      { path: { type: "special", value: { kind: "project_roots", subpath: "secrets" } }, access: "deny" },
    ],
  },
};
const permissionParams = { environmentId: "local", startedAtMs: 0, cwd: "/work/example-app", reason: "needs access", permissions };
const noNetwork = { permissions: {}, scope: "turn" };
const question = { id: "q1", header: "Note", question: "Enter a note", isOther: false, isSecret: false, options: null };

const cases = [
  {
    name: "permissions",
    method: "item/permissions/requestApproval",
    params: permissionParams,
    answer: async (card) => {
      const detail = await card.getByTestId("approval-permissions").textContent();
      check("the permission card exposes network, every path and access mode",
        ["네트워크", "/workspace/source", "/workspace/build/**", "project_roots", "secrets", "(read)", "(write)", "(deny)"].every((text) => detail.includes(text)));
      await card.getByTestId("approval-acceptForSession").click();
    },
    expected: { permissions, scope: "session" },
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
      await card.locator("label", { hasText: "Page" }).locator("select").selectOption({ label: "b" });
      await card.locator("label", { hasText: "Count" }).locator("input").fill("3");
      await card.getByTestId("elicitation-accept").click();
    },
    expected: { action: "accept", content: { page: "b", count: 3 }, _meta: null },
  },
  {
    name: "constrained MCP elicitation",
    method: "mcpServer/elicitation/request",
    params: { serverName: "fixture", mode: "form", _meta: null, message: "Constrained values", requestedSchema: {
      type: "object", properties: {
        count: { type: "integer", title: "Count", minimum: 2, maximum: 4, default: 3 },
        text: { type: "string", title: "Text", minLength: 2, maxLength: 3, default: "ok" },
        tags: { type: "array", title: "Tags", minItems: 1, maxItems: 2, items: { type: "string", enum: ["a", "b", "c"] } },
      }, required: ["count", "text", "tags"],
    } },
    answer: async (card, _page, gateway) => {
      const reject = async (label) => {
        await card.getByTestId("elicitation-accept").click();
        await card.getByTestId("elicitation-error").waitFor({ state: "visible" });
        check(label + " keeps the request open without responding", gateway.answeredRequests.length === 0 && await card.count() === 1);
      };
      await reject("minItems");
      for (const option of ["a", "b", "c"]) await card.getByRole("checkbox", { name: option, exact: true }).check();
      await reject("maxItems");
      await card.getByRole("checkbox", { name: "c", exact: true }).uncheck();
      await card.locator("label", { hasText: "Count" }).locator("input").fill("1");
      await reject("minimum");
      await card.locator("label", { hasText: "Count" }).locator("input").fill("3");
      await card.locator("label", { hasText: "Text" }).locator("input").fill("x");
      await reject("minLength");
      await card.locator("label", { hasText: "Text" }).locator("input").fill("😀😀");
      await card.getByTestId("elicitation-accept").click();
    },
    expected: { action: "accept", content: { count: 3, text: "😀😀", tags: ["a", "b"] }, _meta: null },
  },
  {
    name: "MCP empty values and defaults",
    method: "mcpServer/elicitation/request",
    params: { serverName: "fixture", mode: "form", _meta: null, message: "Default values", requestedSchema: {
      type: "object", properties: {
        empty: { type: "string" },
        array: { type: "array", items: { enum: ["a"] } },
        selection: { type: "string", enum: ["", "x"], default: "" },
        flag: { type: "boolean", default: false },
        count: { type: "integer", default: 0, minimum: null, maximum: null },
        optionalText: { type: "string", minLength: 2 },
        optionalTags: { type: "array", minItems: 1, items: { enum: ["a"] } },
      }, required: ["empty", "array", "selection", "flag", "count"],
    } },
    answer: async (card) => { await card.getByTestId("elicitation-accept").click(); },
    expected: { action: "accept", content: { empty: "", array: [], selection: "", flag: false, count: 0 }, _meta: null },
  },
  {
    name: "unsupported MCP constraint",
    method: "mcpServer/elicitation/request",
    params: { serverName: "fixture", mode: "openai/form", _meta: null, message: "Unsupported pattern", requestedSchema: {
      type: "object", properties: { text: { type: "string", pattern: "^[a-z]+$" } },
    } },
    answer: async (card) => {
      check("unsupported constraints cannot be partially accepted", await card.getByTestId("elicitation-unsupported").count() === 1 && await card.getByTestId("elicitation-accept").count() === 0);
      await card.getByTestId("elicitation-cancel").click();
    },
    expected: { action: "cancel", content: null, _meta: null },
  },
  {
    name: "concurrent approval and forms",
    method: "item/permissions/requestApproval",
    params: permissionParams,
    additionalRequests: [
      { id: "review-permission-second", method: "item/permissions/requestApproval", params: permissionParams },
      { id: "review-question", method: "item/tool/requestUserInput", params: { isBlocking: true, autoResolutionMs: null, questions: [question] } },
      { id: "review-elicitation", method: "mcpServer/elicitation/request", params: { serverName: "fixture", mode: "form", _meta: null, message: "Enter metadata", requestedSchema: { type: "object", properties: { note: { type: "string", title: "Metadata" } } } } },
    ],
    answer: async (_cards, page, gateway) => {
      const settle = () => page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
      await page.getByTestId("user-input-text").press("Control+Enter");
      await settle();
      check("Ctrl+Enter in a user-input form never grants the earlier permission card", await page.getByTestId("approval-permissions").count() === 2
        && gateway.answeredRequests.every((reply) => reply.requestId === "review-question"));
      await page.locator("label", { hasText: "Metadata" }).locator("input").press("Control+Backspace");
      await settle();
      check("Ctrl+Backspace in an empty elicitation form never rejects the earlier permission card", await page.getByTestId("approval-permissions").count() === 2
        && gateway.answeredRequests.every((reply) => reply.requestId === "review-question"));
      // 폼 밖의 기존 승인/거부 단축키는 유지한다.
      await page.locator("label", { hasText: "Metadata" }).locator("input").evaluate((input) => input.blur());
      await page.keyboard.press("Control+Enter");
      await settle();
      check("Ctrl+Enter outside forms still accepts the first permission card", await page.getByTestId("approval-permissions").count() === 1);
      await page.keyboard.press("Control+Backspace");
      await settle();
      check("Ctrl+Backspace outside forms still declines the next permission card", await page.getByTestId("approval-permissions").count() === 0);
      if (await page.getByTestId("user-input-skip").count()) await page.getByTestId("user-input-skip").click();
      await page.getByTestId("elicitation-cancel").click();
    },
    expectedById: (first) => ({
      [first]: { permissions, scope: "turn" },
      "review-permission-second": noNetwork,
      "review-question": { answers: {} },
      "review-elicitation": { action: "cancel", content: null, _meta: null },
    }),
  },
];

const folder = mkdtempSync(join(tmpdir(), "akra-e2e-server-requests-"));
try {
  for (const testCase of cases) {
    const userData = mkdtempSync(join(folder, "user-data-"));
    const endpoint = fake.uniqueTestEndpoint();
    const recording = recordingWith(folder, testCase.method, testCase.params, testCase.additionalRequests);
    const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, recording.path]);
    const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
    delete env.ELECTRON_RUN_AS_NODE;
    const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
    try {
      const page = await app.firstWindow();
      await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
      await page.getByTestId("thread-entry").first().click();
      const card = page.getByTestId("approval");
      await card.first().waitFor({ timeout: 15_000 });
      check(`the ${testCase.name} request shows a card above the composer`, (await page.getByTestId("approvals").count()) === 1);
      await testCase.answer(card, page, gateway);
      await card.first().waitFor({ state: "detached", timeout: 5_000 });
      const expectedById = testCase.expectedById?.(recording.requestId) ?? { [recording.requestId]: testCase.expected };
      const deadline = Date.now() + 5_000;
      while (gateway.answeredRequests.length < Object.keys(expectedById).length && Date.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 50));
      const actualById = Object.fromEntries(gateway.answeredRequests.map((reply) => [reply.requestId, reply.result]));
      check(`the ${testCase.name} answers reach the matching server requests`,
        Object.keys(actualById).length === Object.keys(expectedById).length
        && Object.entries(expectedById).every(([id, result]) => JSON.stringify(actualById[id]) === JSON.stringify(result)),
        JSON.stringify(actualById));
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
