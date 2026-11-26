// 화면 시험 (#2841): 사이드바 스레드의 오른쪽 클릭 메뉴로 이름 바꾸기, 보관과 되돌리기를 공식 app-server 요청으로 보낸다.
// 실제 codex 와 턴은 쓰지 않는다 (가짜 Gateway). userData 는 임시 폴더를 쓴다.
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
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

// 녹화의 스레드 하나이다. 보관을 되돌리는 응답으로 돌려준다.
const listed = readFileSync(fake.APPROVAL_THREAD_LIST_RECORDING, "utf8").split(/\r?\n/).filter(Boolean)
  .map((line) => JSON.parse(JSON.parse(line).text)).find((message) => message.result?.data)?.result.data[0];
const requests = [];
const userData = mkdtempSync(join(tmpdir(), "akra-e2e-thread-menu-"));
const endpoint = fake.uniqueTestEndpoint();
// 녹화 둘: 스레드 목록과, initialize 응답이 있는 턴 녹화이다.
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  appServer(method, params) {
    if (!["thread/name/set", "thread/archive", "thread/unarchive"].includes(method)) return undefined;
    requests.push({ method, params });
    return { result: method === "thread/unarchive" ? { thread: listed } : {} };
  },
});
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;

const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  const entry = page.getByTestId("thread-entry").first();
  await entry.waitFor({ timeout: 10_000 });

  await entry.click({ button: "right" });
  await page.getByTestId("thread-menu").waitFor({ timeout: 5_000 });
  check("the menu offers the copy items", (await page.getByTestId("thread-menu-copy-id").count()) === 1);
  await page.getByTestId("thread-menu-rename").click();
  await page.getByTestId("thread-rename-input").fill("새 이름");
  await page.getByTestId("thread-rename-input").press("Enter");
  await page.getByTestId("thread-entry").filter({ hasText: "새 이름" }).waitFor({ timeout: 5_000 });
  check("rename sends thread/name/set and shows the new title",
    JSON.stringify(requests.at(-1)) === JSON.stringify({ method: "thread/name/set", params: { threadId: listed.id, name: "새 이름" } }), JSON.stringify(requests));

  await page.getByTestId("thread-entry").first().click({ button: "right" });
  await page.keyboard.press("Escape");
  check("escape closes the menu", (await page.getByTestId("thread-menu").count()) === 0);

  await page.getByTestId("thread-entry").first().click({ button: "right" });
  await page.getByTestId("thread-menu-archive").click();
  await page.getByTestId("archived-notice").waitFor({ timeout: 5_000 });
  check("archive sends thread/archive and removes the thread", requests.at(-1)?.method === "thread/archive"
    && (await page.getByTestId("thread-entry").count()) === 0, JSON.stringify(requests.at(-1)));
  await page.getByTestId("archived-undo").click();
  await page.getByTestId("thread-entry").first().waitFor({ timeout: 5_000 });
  check("undo sends thread/unarchive and brings the thread back", requests.at(-1)?.method === "thread/unarchive");
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
