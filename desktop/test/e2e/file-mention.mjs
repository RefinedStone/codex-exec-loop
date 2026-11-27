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
const { PALETTE_ORDER } = await import(pathToFileURL(join(desktopDir, "src/shared/commands.ts")).href);

const failures = [];
const check = (name, ok, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${!ok && detail ? ` | ${detail}` : ""}`);
  if (!ok) failures.push(name);
};

const searches = [];
const modelCalls = [];
const userData = mkdtempSync(join(tmpdir(), "akra-e2e-file-mention-"));
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  appServer(method, params) {
    if (method === "turn/start" || method === "turn/steer") modelCalls.push({ method, params });
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

  // 후보를 연 뒤 커서를 언급 밖으로 옮기면 후보를 닫고 뒤의 글을 지우지 않는다.
  const tail = "read @main and preserve this instruction";
  await input.fill("read  and preserve this instruction");
  await page.keyboard.press("Home");
  for (let step = 0; step < 5; step += 1) await page.keyboard.press("ArrowRight");
  await page.keyboard.type("@main");
  await page.getByTestId("mention-list").waitFor({ timeout: 5_000 });
  await page.keyboard.press("Control+End");
  await page.waitForTimeout(400);
  check("moving the caret out of the mention closes the list", (await page.getByTestId("mention-list").count()) === 0);
  check("the text after the mention stays", (await input.inputValue()) === tail, await input.inputValue());

  // 예약으로 초안을 비우면 후보도 사라지고 늦게 온 찾기가 다시 열지 않는다.
  await input.fill("");
  await page.keyboard.type("@main");
  await page.getByTestId("mention-list").waitFor({ timeout: 5_000 });
  await page.keyboard.type(".");
  await page.getByTestId("composer-queue").click();
  await page.waitForTimeout(400);
  check("reserving the draft clears the candidates", (await input.inputValue()) === "" && (await page.getByTestId("mention-list").count()) === 0, await input.inputValue());


  // 앞의 예약 시험이 보낸 가짜 턴은 제외하고, 명령을 고르는 동안의 전송을 센다.
  const modelCallsBeforeCommands = modelCalls.length;
  // 같은 명령 표의 후보를 키보드/클릭으로 실행한다. 명령 선택은 모델 전송이 아니다.
  await input.fill("/");
  const slashList = page.getByTestId("slash-command-list");
  await slashList.waitFor();
  check("/ shows shared app commands", (await slashList.getByRole("option").count()) === PALETTE_ORDER.length);
  await page.keyboard.press("ArrowDown");
  const selectionVisible = await slashList.evaluate((list) => {
    const selected = list.querySelector("button.selected");
    const other = list.querySelector("button:not(.selected)");
    return selected && other && getComputedStyle(selected).backgroundColor !== getComputedStyle(other).backgroundColor;
  });
  check("the selected candidate has a visible background", selectionVisible);

  if (process.env.AKRA_E2E_ARTIFACT_DIR) await page.screenshot({ path: join(process.env.AKRA_E2E_ARTIFACT_DIR, "slash-commands.png") });
  await input.fill("/sett");
  await slashList.getByRole("button", { name: "/settings 설정" }).waitFor();
  await page.keyboard.press("Enter");
  await page.getByTestId("settings").waitFor();
  check("Enter runs the app command and clears only the command draft", (await input.inputValue()) === "");
  await page.getByTestId("settings").getByTitle("닫기").click();
  // 포커스만 나갔다가 돌아오면 같은 커서에서도 후보를 복원한다. 모델로 보내지 않는다.
  await input.fill("/settings");
  await slashList.waitFor();
  await page.getByTestId("composer-send").focus();
  await slashList.waitFor({ state: "hidden" });
  await input.focus();
  await slashList.waitFor();
  await page.keyboard.press("Enter");
  await page.getByTestId("settings").waitFor();
  check("refocusing restores commands and Enter runs settings", (await input.inputValue()) === "");
  check("refocusing a command does not send a model request", modelCalls.length === modelCallsBeforeCommands);
  await page.getByTestId("settings").getByTitle("닫기").click();

  await input.fill("/settings");
  await slashList.waitFor();
  await page.keyboard.press("Escape");
  check("Escape preserves the draft and dismisses commands", (await input.inputValue()) === "/settings" && (await slashList.count()) === 0);
  await page.getByTestId("composer-send").focus();
  await input.focus();
  check("Escape dismissal survives a focus roundtrip", (await input.inputValue()) === "/settings" && (await slashList.count()) === 0);

  await input.fill("/review");
  await slashList.waitFor();
  await page.keyboard.press("Shift+Enter");
  check("Shift+Enter writes a newline without running a command", (await input.inputValue()) === "/review\n" && (await slashList.count()) === 0);
  await input.fill("/usr/local");
  check("absolute paths stay ordinary input", (await slashList.count()) === 0);
  await input.fill("read /settings");
  check("slashes in prose stay ordinary input", (await slashList.count()) === 0);
  await input.fill("/settings");
  await slashList.waitFor();
  await input.evaluate((element) => {
    element.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", keyCode: 229, bubbles: true }));
  });
  check("IME confirmation cannot run a command", (await input.inputValue()) === "/settings" && (await page.getByTestId("settings").count()) === 0);
  await page.keyboard.press("Home");
  await page.waitForTimeout(50);
  check("moving the caret away dismisses commands", (await slashList.count()) === 0);
  await page.keyboard.press("End");
  await slashList.waitFor();
  await slashList.getByRole("button", { name: "/settings 설정" }).click();
  await page.getByTestId("settings").waitFor();
  check("click runs the shared command", (await input.inputValue()) === "");
  await page.getByTestId("settings").getByTitle("닫기").click();

  await input.fill("/theme");
  await slashList.waitFor();
  await page.keyboard.press(process.platform === "darwin" ? "Meta+n" : "Control+n");
  await page.getByTestId("home").waitFor();
  await page.getByTestId("composer-input").waitFor();
  check("command candidates do not follow navigation to home", (await slashList.count()) === 0);
  check("slash interactions made no model requests", modelCalls.length === modelCallsBeforeCommands, JSON.stringify(modelCalls.slice(modelCallsBeforeCommands)));

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
