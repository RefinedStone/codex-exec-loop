// 화면 시험 (R2): 터미널을 열고 명령을 실행하고 탭을 닫는다. 실제 codex 와 턴은 쓰지 않는다 (가짜 Gateway).
// Windows 에서는 건너뛴다. 실제 셸을 띄우면 사용자 화면에 창이 뜰 수 있다. AKRA_E2E_TERMINAL=1 이면 Windows 에서도 돈다.
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

if (process.platform === "win32" && process.env.AKRA_E2E_TERMINAL !== "1") {
  console.log("SKIP terminal e2e on Windows (set AKRA_E2E_TERMINAL=1 to run)");
  process.exit(0);
}

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);

const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.BASIC_RECORDING]);
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1" };
delete env.ELECTRON_RUN_AS_NODE;

const failures = [];
const check = (name, ok) => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}`);
  if (!ok) failures.push(name);
};

/** WebGL 로 그리면 DOM 에 글자가 없으므로 xterm 버퍼를 읽는다. */
const bufferText = (page, index) =>
  page.evaluate((at) => {
    const terminal = document.querySelectorAll('[data-testid="terminal-view"]')[at]?.akraTerminal;
    if (!terminal) return "";
    const buffer = terminal.buffer.active;
    const lines = [];
    for (let line = 0; line < buffer.length; line += 1) lines.push(buffer.getLine(line)?.translateToString(true) ?? "");
    return lines.join("\n");
  }, index);

const waitForText = async (page, index, text, timeoutMs) => {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if ((await bufferText(page, index)).includes(text)) return true;
    await page.waitForTimeout(200);
  }
  return false;
};

const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });

  await page.getByTestId("terminal-toggle").click();
  await page.getByTestId("terminal-view").first().waitFor({ timeout: 15_000 });
  check("the toggle opens a terminal tab", (await page.getByTestId("terminal-tab").count()) === 1);

  // 셸이 준비되기 전에 친 글자도 셸이 받는다. 결과 줄이 나올 때까지 기다린다.
  await page.waitForTimeout(500);
  await page.locator('[data-testid="terminal-view"] textarea').first().focus();
  await page.keyboard.type("echo akra-term-$((40+2))\r");
  check("a typed command runs in the shell", await waitForText(page, 0, "akra-term-42", 15_000));

  // 포커스가 터미널에 있으면 메뉴의 확대는 터미널만 바꾼다. 글자 크기에 곱하고 열과 행을 다시 맞춘다 (#2629).
  const terminalSize = () => page.getByTestId("terminal-view").first().evaluate((element) => ({ cols: element.akraTerminal.cols, fontSize: element.akraTerminal.options.fontSize }));
  const clickMenu = (id) => app.evaluate(({ BrowserWindow, Menu }, itemId) => Menu.getApplicationMenu()?.getMenuItemById(itemId)?.click(undefined, BrowserWindow.getAllWindows()[0]), id);
  const before = await terminalSize();
  await clickMenu("zoom-in");
  await clickMenu("zoom-in");
  await clickMenu("zoom-in");
  await page.waitForFunction((size) => document.querySelector('[data-testid="terminal-view"]').akraTerminal.options.fontSize > size, before.fontSize, { timeout: 5_000 }).catch(() => undefined);
  const zoomed = await terminalSize();
  const zooms = await page.evaluate(() => window.akra.zoom.get());
  check("the zoom shortcut in the terminal zooms only the terminal", zooms.terminal === 1.5 && zooms.home === undefined, JSON.stringify(zooms));
  check("the terminal font grows and the columns fit again", zoomed.fontSize > before.fontSize && zoomed.cols < before.cols, JSON.stringify({ before, zoomed }));
  await clickMenu("zoom-reset");
  await page.waitForFunction((size) => document.querySelector('[data-testid="terminal-view"]').akraTerminal.options.fontSize === size, before.fontSize, { timeout: 5_000 }).catch(() => undefined);
  check("the terminal zoom resets", (await terminalSize()).cols === before.cols);

  await page.getByTestId("terminal-add").click();
  await page.waitForFunction(() => document.querySelectorAll('[data-testid="terminal-view"]').length === 2);
  check("a second tab opens its own shell", (await page.getByTestId("terminal-tab").count()) === 2);

  await page.locator('[data-testid="terminal-view"]:not([hidden]) textarea').focus();
  await page.keyboard.type("exit\r");
  await page.getByTestId("terminal-tab").filter({ hasText: "(끝남)" }).waitFor({ timeout: 15_000 });
  check("exiting the shell marks the tab as ended", true);

  await page.getByTestId("terminal-close").first().click();
  await page.getByTestId("terminal-close").first().click();
  check("closing every tab hides the terminal", (await page.getByTestId("terminal-view").count()) === 0);

  // Reload reuses the same Electron WebContents. The old PTY must not outlive
  // the renderer that owned it even though the window itself was not closed.
  await page.getByTestId("terminal-toggle").click();
  await page.getByTestId("terminal-view").first().waitFor({ timeout: 15_000 });
  await page.locator('[data-testid="terminal-view"] textarea').first().focus();
  await page.keyboard.type("echo akra-reload-shell-$$\r");
  let oldShellPid = null;
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    const found = (await bufferText(page, 0)).match(/akra-reload-shell-(\d+)/);
    if (found) {
      oldShellPid = Number(found[1]);
      break;
    }
    await page.waitForTimeout(100);
  }
  if (process.platform !== "win32") check("reload test recorded the old shell pid", Number.isInteger(oldShellPid) && oldShellPid > 1);
  await page.reload();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  check("renderer reload clears the old terminal tabs", (await page.getByTestId("terminal-view").count()) === 0);
  if (process.platform !== "win32" && oldShellPid) {
    const expired = Date.now() + 5_000;
    let gone = false;
    while (Date.now() < expired) {
      try {
        process.kill(oldShellPid, 0);
      } catch (error) {
        if (error.code === "ESRCH") {
          gone = true;
          break;
        }
        throw error;
      }
      await page.waitForTimeout(100);
    }
    check("renderer reload terminates its old PTY process", gone);
  }
} catch (error) {
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  const exited = new Promise((resolve) => app.process().once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  await gateway.close();
}
process.exit(failures.length === 0 ? 0 : 1);
