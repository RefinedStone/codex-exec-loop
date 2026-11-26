// 화면 시험 (#2837): 입력창에서 `@` 를 치면 대화 폴더의 파일 후보가 보이고, 고른 경로가 입력에 들어간다.
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

const searches = [];
const userData = mkdtempSync(join(tmpdir(), "akra-e2e-file-mention-"));
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  appServer(method, params) {
    if (method !== "fuzzyFileSearch") return undefined;
    searches.push(params);
    const files = ["src/main.ts", "src/main.test.ts"].filter((path) => path.includes(params.query))
      .map((path) => ({ root: params.roots[0], path, match_type: "file", file_name: path.split("/").pop(), score: 1, indices: null }));
    return { result: { files } };
  },
});
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;

const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  const input = page.getByTestId("composer-input");
  await input.waitFor({ timeout: 15_000 });
  await input.click();
  await page.keyboard.type("look at @main");
  await page.getByTestId("mention-list").waitFor({ timeout: 5_000 });
  check("@ shows candidates from the thread folder", searches.at(-1)?.roots?.[0] === "/work/example-app" && searches.at(-1)?.query === "main", JSON.stringify(searches.at(-1)));
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  check("the chosen path replaces the @word", (await input.inputValue()) === "look at src/main.test.ts ", await input.inputValue());
  check("choosing does not send the message", (await page.getByTestId("mention-list").count()) === 0 && (await input.inputValue()).length > 0);
  await page.keyboard.type("@zzz");
  await page.waitForTimeout(400);
  check("no list when nothing matches", (await page.getByTestId("mention-list").count()) === 0);
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
