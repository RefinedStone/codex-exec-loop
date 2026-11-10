// 화면 시험 (plan 21 3단계): 빌드한 앱을 가짜 Gateway 와 가짜 MCP hub 에 붙여 Notion 패널을 확인한다. 실제 codex,
// Notion, 턴은 쓰지 않는다. 열면 최근 페이지를 읽고, 검색하고, "스레드로 넘기기" 로 새 스레드의 입력창을 채운다.
// 창은 숨긴 채로 돌리고 패널의 글을 출력한다 (숨긴 창 텍스트 캡처).
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);

const textResult = (value) => ({ content: [{ type: "text", text: JSON.stringify(value) }], isError: false });
const mcpHub = {
  statuses: [
    {
      name: "notion",
      auth: "oAuth",
      connection: "connected",
      tools: [{ name: "notion-search" }, { name: "notion-fetch" }, { name: "notion-list-recent-pages" }],
      error: null,
    },
  ],
  tools: {
    "notion-list-recent-pages": textResult({
      results: [
        { type: "page", title: "Sample roadmap", url: "https://www.notion.so/Sample-roadmap-1" },
        { type: "database", title: "Sample tasks", url: "https://www.notion.so/Sample-tasks-2" },
      ],
    }),
    "notion-search": { content: [], structuredContent: { results: [{ id: "3", title: "Sample spec", url: "https://www.notion.so/Sample-spec-3" }] } },
    "notion-fetch": textResult({
      id: "3",
      title: "Sample spec",
      url: "https://www.notion.so/Sample-spec-3",
      text: "# Goals\n- ship the sample",
      path: "Team / Sample spec",
    }),
  },
};

const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(
  endpoint,
  [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING, fake.THREAD_START_RECORDING],
  { mcpHub },
);
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
  // 넘기기는 열린 스레드의 폴더에 새 스레드를 만든다. 폴더 선택 대화상자를 피하려고 스레드를 먼저 연다.
  await page.getByTestId("thread-entry").first().click();

  await page.getByTestId("notion-toggle").click();
  await page.getByTestId("notion-panel").waitFor({ timeout: 10_000 });
  await page.getByTestId("notion-page").first().waitFor({ timeout: 10_000 });
  check("the panel lists recently viewed pages", (await page.getByTestId("notion-page").count()) === 2);
  check("the panel asked the hub for the notion server", gateway.receivedMethods.includes("akra/mcp/statuses"));

  await page.getByLabel("Notion 검색").fill("spec");
  await page.getByTestId("notion-search").getByRole("button", { name: "검색" }).click();
  await page.getByText("검색: spec (1)").waitFor({ timeout: 10_000 });
  check("search shows the matching page", (await page.getByTestId("notion-page").first().textContent())?.includes("Sample spec") === true);

  await page.getByTestId("notion-hand-off").first().click();
  const composer = page.getByTestId("composer-input");
  const deadline = Date.now() + 10_000;
  let draft = "";
  while (Date.now() < deadline) {
    draft = await composer.inputValue();
    if (draft.includes("Sample spec")) break;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  check("hand-off fills the new thread's composer with the page", draft.includes("# Goals") && draft.includes("https://www.notion.so/Sample-spec-3"));
  check("hand-off does not send the message", !gateway.receivedMethods.includes("turn/start"));

  console.log("--- notion panel text ---");
  console.log(await page.getByTestId("notion-panel").innerText());
  console.log("-------------------------");
} catch (error) {
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  // 창을 닫으면 앱이 끝난다. app.close() 는 쓰지 않는다 (P2 기록 참고).
  const exited = new Promise((resolve) => app.process().once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  await gateway.close();
}

process.exit(failures.length === 0 ? 0 : 1);
