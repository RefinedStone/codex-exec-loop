// 화면 시험 (#2842): 대화의 오른쪽 클릭 메뉴가 고른 글을 입력창 초안에 인용으로 붙이고, 기존 초안을 지우지 않는다.
// 실제 codex 와 턴은 쓰지 않는다 (가짜 Gateway). userData 는 임시 폴더를 쓴다.
import { mkdtempSync, rmSync } from "node:fs";
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

const userData = mkdtempSync(join(tmpdir(), "akra-e2e-conversation-menu-"));
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.APPROVAL_TURN_RECORDING]);
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;

const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  const message = page.getByTestId("item-user").first();
  await message.waitFor({ timeout: 15_000 });
  const input = page.getByTestId("composer-input");
  await input.fill("내 메모");

  // 메시지의 글을 고르고 그 위에서 오른쪽 클릭한다.
  const chosen = await message.locator("p").evaluate((element) => {
    const range = document.createRange();
    range.selectNodeContents(element);
    const selection = window.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
    return selection.toString();
  });
  await message.locator("p").click({ button: "right" });
  await page.getByTestId("conversation-menu").waitFor({ timeout: 5_000 });
  check("the menu offers message copy and find", (await page.getByTestId("conversation-menu-copy-message").count()) === 1
    && (await page.getByTestId("conversation-menu-find").count()) === 1);
  await page.getByTestId("conversation-menu-quote").click();
  const draft = await input.inputValue();
  check("the quote follows the existing draft", draft.startsWith("내 메모\n\n> ") && draft.includes(chosen.split("\n")[0]), JSON.stringify(draft));

  await message.locator("p").click({ button: "right" });
  await page.getByTestId("conversation-menu-find").click();
  await page.getByTestId("find-input").waitFor({ timeout: 5_000 });
  check("find opens the find bar", true);
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
