// 화면 시험 (plan 26 U2): 설정 화면을 열고, 글자 크기·알림·잠자기 방지를 바꾸고, 저장과 적용을 확인한다.
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

const userData = mkdtempSync(join(tmpdir(), "akra-e2e-settings-"));
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.BASIC_RECORDING]);
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;

const failures = [];
const check = (name, ok, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${!ok && detail ? ` | ${detail}` : ""}`);
  if (!ok) failures.push(name);
};
const saved = () => {
  try {
    return JSON.parse(readFileSync(join(userData, "app-settings.json"), "utf8"));
  } catch {
    return null;
  }
};
const savedWhen = async (page, done) => {
  for (let attempt = 0; attempt < 30; attempt += 1) {
    const settings = saved();
    if (settings && done(settings)) return true;
    await page.waitForTimeout(100);
  }
  return false;
};
const launch = () => electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
const quit = async (app) => {
  // 프로세스는 앱이 끝나기 전에 한 번만 받는다. 앱이 끝난 뒤의 `app.process()` 는 오류이다: playwright 가 그 객체를 지운다.
  const child = app.process();
  const exited = new Promise((resolve) => child.once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  // macOS 에서는 창을 모두 닫아도 앱이 남는다.
  if (child.exitCode === null) child.kill();
};
const token = (page, name) => page.evaluate((property) => document.documentElement.style.getPropertyValue(property), name);

let app = await launch();
try {
  let page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });

  // 앱 메뉴의 "설정…" 이 설정 화면을 연다.
  await app.evaluate(({ Menu }) => Menu.getApplicationMenu()?.getMenuItemById("open-settings")?.click());
  await page.getByTestId("settings").waitFor({ timeout: 5_000 });
  check("the app menu opens the settings", true);

  // 글자 크기: 대화 글자를 두 단계 키운다.
  const reading = page.getByTestId("settings-font-reading");
  await reading.getByRole("button", { name: "대화 글자 크기 늘리기" }).click();
  await reading.getByRole("button", { name: "대화 글자 크기 늘리기" }).click();
  check("the reading size steps up", (await page.getByTestId("settings-font-reading-value").textContent()) === "17px");
  check("the reading size reaches the CSS token", (await token(page, "--text-reading")) === "17px");
  await page.getByTestId("settings-font-ui").getByRole("button", { name: "UI 글자 크기 줄이기" }).click();
  check("the UI size moves the UI tokens together", (await token(page, "--text-m")) === "12px" && (await token(page, "--text-s")) === "11px");

  // 알림과 잠자기 방지.
  await page.getByTestId("settings-notify-mode").locator('[data-value="always"]').click();
  await page.getByTestId("settings-notify-sound").uncheck();
  await page.getByTestId("settings-keep-awake").locator('[data-value="working"]').click();
  check(
    "main saves the settings in the userData folder",
    await savedWhen(page, (settings) => settings.fonts.reading === 17 && settings.fonts.ui === 12 && settings.notifications.mode === "always" && settings.notifications.sound === false && settings.keepAwake === "working"),
    JSON.stringify(saved()),
  );

  // 설정 찾기.
  await page.getByTestId("settings-search").fill("터미널");
  check("search keeps only the matching setting", (await page.locator('[data-testid^="settings-row-"]').count()) === 1 && (await page.getByTestId("settings-row-font-terminal").count()) === 1);
  await page.getByTestId("settings-search").fill("akra-no-such-setting");
  check("search reports no match", (await page.locator('[data-testid^="settings-row-"]').count()) === 0);
  await page.keyboard.press("Escape");
  check("escape closes the settings", (await page.getByTestId("settings").count()) === 0);

  // 다시 실행하면 저장한 값으로 시작한다.
  await quit(app);
  app = await launch();
  page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.waitForFunction(() => document.documentElement.style.getPropertyValue("--text-reading") === "17px", null, { timeout: 5_000 });
  await page.getByTestId("settings-open").click();
  check("a restart keeps the settings", (await page.getByTestId("settings-keep-awake").locator(".selected").getAttribute("data-value")) === "working");
  // 값을 누르면 기본값으로 돌아간다.
  await page.getByTestId("settings-font-reading-value").click();
  check("clicking a value restores its default", (await page.getByTestId("settings-font-reading-value").textContent()) === "15px");
} catch (error) {
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  await quit(app);
  await gateway.close();
  rmSync(userData, { recursive: true, force: true, maxRetries: 5 });
}
process.exit(failures.length === 0 ? 0 : 1);
