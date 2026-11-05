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
  await go("/echo");
  check("a new profile has no cookie from the default profile", await titleIs("cookie:"), await slot.getAttribute("title"));

  await page.getByTestId("browser-profile").selectOption("default");
  await page.waitForTimeout(500);
  await go("/echo");
  check("switching back finds the default profile's cookie", await titleIs("cookie:akra=1"), await slot.getAttribute("title"));

  await page.getByTestId("browser-save-preset").click();
  await page.getByTestId("browser-editing-input").fill("Echo");
  await page.getByTestId("browser-editing-input").press("Enter");
  await page.locator('[data-testid="browser-preset"] option', { hasText: "Echo" }).waitFor({ state: "attached", timeout: 10_000 });
  await page.getByTestId("browser-profile").selectOption("qa");
  await page.waitForTimeout(500);
  await page.getByTestId("browser-preset").selectOption("Echo");
  await page.waitForFunction(() => document.querySelector('[data-testid="browser-profile"]')?.value === "default", null, { timeout: 10_000 });
  check("a preset switches to its profile and opens its address", await titleIs("cookie:akra=1"), await slot.getAttribute("title"));

  await page.getByTestId("browser-address").fill("file:///etc/passwd");
  await page.getByTestId("browser-address").press("Enter");
  await page.getByTestId("browser-message").waitFor({ timeout: 5_000 });
  check("non-http addresses are refused", true);

  const saved = JSON.parse(readFileSync(join(userData, "browser-settings.json"), "utf8"));
  check("profiles are saved in the userData folder", saved.profiles.some((profile) => profile.id === "qa"));

  await page.getByTestId("browser-toggle").click();
  check("closing the panel removes it", (await page.getByTestId("browser-panel").count()) === 0);
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
