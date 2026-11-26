// 화면 시험 (R3): 브라우저 패널로 로컬 시험 서버를 열고, 프로필마다 쿠키가 따로 있는지 확인한다.
// 실제 codex 와 턴은 쓰지 않는다 (가짜 Gateway). userData 는 임시 폴더를 쓴다 (사용자 앱 데이터에 쓰지 않는다).
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);

// /set 은 쿠키를 심고, /echo 는 받은 쿠키를 제목에 보여 준다.
const site = createServer((request, response) => {
  const page = (title) => `<!doctype html><title>${title}</title><p>${title}</p>`;
  if (request.url === "/set") {
    response.writeHead(200, { "content-type": "text/html", "set-cookie": "akra=1; Path=/; Max-Age=3600" });
    response.end(page("set"));
  } else {
    response.writeHead(200, { "content-type": "text/html" });
    response.end(page(`cookie:${request.headers.cookie ?? ""}`));
  }
});
await new Promise((resolve) => site.listen(0, "127.0.0.1", resolve));
const origin = `127.0.0.1:${site.address().port}`;

const userData = mkdtempSync(join(tmpdir(), "akra-e2e-browser-"));
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.BASIC_RECORDING]);
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;

const failures = [];
const check = (name, ok, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${!ok && detail ? ` | ${detail}` : ""}`);
  if (!ok) failures.push(name);
};

const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("browser-toggle").click();
  await page.getByTestId("browser-panel").waitFor({ timeout: 10_000 });

  const slot = page.getByTestId("browser-slot");
  // The renderer selector changes optimistically; the main process returns
  // BrowserState only after the target partition's view has been installed.
  // Await that observed profile instead of an unrelated web-contents event.
  const waitForAppliedProfile = (profileId) =>
    page.waitForFunction(
      (expected) => document.querySelector('[data-testid="browser-slot"]')?.getAttribute("data-active-profile") === expected,
      profileId,
      { timeout: 10_000 },
    );
  const go = async (path) => {
    await page.getByTestId("browser-address").fill(`${origin}${path}`);
    await page.getByTestId("browser-address").press("Enter");
  };
  const titleIs = async (title) => {
    try {
      await page.waitForFunction((expected) => document.querySelector('[data-testid="browser-slot"]')?.getAttribute("title") === expected, title, { timeout: 15_000 });
      return true;
    } catch {
      return false;
    }
  };

  await go("/set");
  check("the panel opens a page", await titleIs("set"), await slot.getAttribute("title"));
  await go("/echo");
  check("the default profile keeps its cookie", await titleIs("cookie:akra=1"), await slot.getAttribute("title"));

  await page.getByTestId("browser-add-profile").click();
  await page.getByTestId("browser-editing-input").fill("QA");
  await page.getByTestId("browser-editing-input").press("Enter");
  await page.waitForFunction(() => document.querySelector('[data-testid="browser-profile"]')?.value === "qa", null, { timeout: 10_000 });
  await waitForAppliedProfile("qa");
  await go("/echo");
  check("a new profile has no cookie from the default profile", await titleIs("cookie:"), await slot.getAttribute("title"));

  await page.getByTestId("browser-profile").selectOption("default");
  await waitForAppliedProfile("default");
  await go("/echo");
  check("switching back finds the default profile's cookie", await titleIs("cookie:akra=1"), await slot.getAttribute("title"));

  await page.getByTestId("browser-save-preset").click();
  await page.getByTestId("browser-editing-input").fill("Echo");
  await page.getByTestId("browser-editing-input").press("Enter");
  await page.locator('[data-testid="browser-preset"] option', { hasText: "Echo" }).waitFor({ state: "attached", timeout: 10_000 });
  await page.getByTestId("browser-profile").selectOption("qa");
  await waitForAppliedProfile("qa");
  await page.getByTestId("browser-preset").selectOption("Echo");
  await page.waitForFunction(() => document.querySelector('[data-testid="browser-profile"]')?.value === "default", null, { timeout: 10_000 });
  await waitForAppliedProfile("default");
  check("a preset switches to its profile and opens its address", await titleIs("cookie:akra=1"), await slot.getAttribute("title"));

  await page.getByTestId("browser-address").fill("file:///etc/passwd");
  await page.getByTestId("browser-address").press("Enter");
  await page.getByTestId("browser-message").waitFor({ timeout: 5_000 });
  check("non-http addresses are refused", true);

  // 페이지는 main 의 보기라서 renderer 의 대화 상자 위에 그려진다. 팔레트가 열린 동안 보기를 숨기고, 닫으면 같은 페이지를 다시 보인다.
  const nativeView = () => app.evaluate(({ BrowserWindow }) => {
    const view = BrowserWindow.getAllWindows()[0].contentView.children.find((child) => child.webContents);
    return view ? { visible: view.getVisible(), url: view.webContents.getURL() } : null;
  });
  const viewBecomes = async (visible) => {
    const deadline = Date.now() + 10_000;
    while (Date.now() < deadline) {
      if ((await nativeView())?.visible === visible) return true;
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    return false;
  };
  const before = await nativeView();
  await page.getByRole("button", { name: "검색과 명령" }).click();
  await page.getByTestId("palette").waitFor({ timeout: 10_000 });
  check("an open palette hides the browser view", await viewBecomes(false), JSON.stringify(await nativeView()));
  await page.keyboard.press("Escape");
  await page.getByTestId("palette").waitFor({ state: "detached", timeout: 10_000 });
  check("closing the palette shows the browser view again", await viewBecomes(true), JSON.stringify(await nativeView()));
  check("the browser view keeps its page across the palette", (await nativeView())?.url === before?.url, JSON.stringify({ before, after: await nativeView() }));

  const saved = JSON.parse(readFileSync(join(userData, "browser-settings.json"), "utf8"));
  check("profiles are saved in the userData folder", saved.profiles.some((profile) => profile.id === "qa"));

  // A renderer reload keeps the BrowserWindow / WebContents id alive. Old
  // native WebContentsView instances nevertheless must be destroyed.
  const oldViewIds = await app.evaluate(({ BrowserWindow }) => {
    const window = BrowserWindow.getAllWindows()[0];
    return window.contentView.children
      .filter((view) => view.webContents)
      .map((view) => view.webContents.id);
  });
  check("reload test begins with a native browser view", oldViewIds.length > 0);
  await page.reload();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  const remainingOldViews = await app.evaluate(({ BrowserWindow }, ids) => {
    const window = BrowserWindow.getAllWindows()[0];
    return window.contentView.children.some((view) => view.webContents && ids.includes(view.webContents.id));
  }, oldViewIds);
  check("renderer reload releases the previous native browser view", !remainingOldViews);
} catch (error) {
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  const exited = new Promise((resolve) => app.process().once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  await gateway.close();
  site.close();
  rmSync(userData, { recursive: true, force: true, maxRetries: 5 });
}
process.exit(failures.length === 0 ? 0 : 1);
