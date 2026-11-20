// 화면 시험 (#2427): 로컬 복원본 창을 열고, 자동화로 실행한 앱에서는 열람을 보류하는지, 그 창이 일반 표면과 나뉘어
// 있는지 확인한다. 이 시험은 playwright 가 앱을 조종하므로 실행 프로필이 "자동화" 이다: 복원한 글은 나오지 않는다.
// 실제 codex, 턴, 키 저장소, GitHub 는 쓰지 않는다 (가짜 Gateway). userData 는 임시 폴더를 쓴다.
import { mkdtempSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);

const userData = mkdtempSync(join(tmpdir(), "akra-e2e-vault-restore-"));
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.BASIC_RECORDING]);
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;
const ISSUE = "https://github.com/fixture-owner/fixture-repo/issues/7";

const failures = [];
const check = (name, ok, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${!ok && detail ? ` | ${detail}` : ""}`);
  if (!ok) failures.push(name);
};

const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });

  // 일반 창은 이슈 주소가 아닌 값으로 복원본 창을 열 수 없다.
  const refused = await page.evaluate(() => window.akra.vaultRestore.open("https://example.test/not-an-issue").then(() => false, () => true));
  check("a non-issue address is refused", refused);

  const opened = app.waitForEvent("window");
  await page.evaluate((url) => window.akra.vaultRestore.open(url), ISSUE);
  const restore = await opened;
  await restore.locator('#root[data-phase="held"]').waitFor({ timeout: 10_000 });
  check("the restore window opens with its own page", restore.url().endsWith("vault-restore.html"), restore.url());
  check("viewing is held under automation", (await restore.getByTestId("vault-restore-detail").textContent())?.includes("자동화") === true);
  check("no restored text is on the screen", (await restore.getByTestId("vault-restored-body").count()) === 0);

  // 그 창에는 일반 메시지 버스가 없고, 일반 창에는 복원본 채널이 없다.
  check("the restore window has no general bridge", await restore.evaluate(() => typeof window.akra === "object" && typeof window.akra.send === "undefined" && typeof window.akra.vaultRestore.onState === "function"));
  check("the general window has no restore bridge", await page.evaluate(() => typeof window.akra.vaultRestore.onState === "undefined" && typeof window.akraVaultRestore === "undefined"));

  // 같은 이슈를 다시 열면 창을 하나 더 만들지 않는다.
  await page.evaluate((url) => window.akra.vaultRestore.open(url), ISSUE);
  check("the same issue reuses its window", app.windows().length === 2, String(app.windows().length));

  // 다시 여는 요청도 실행 프로필을 다시 본다.
  await restore.getByTestId("vault-restore-unlock").click();
  await restore.locator('#root[data-phase="held"]').waitFor({ timeout: 5_000 });
  check("another unlock is held again", (await restore.getByTestId("vault-restore-heading").textContent()) === "열람 보류");

  // 프로젝트 전환 알림은 그 창을 잠근다.
  await page.evaluate(() => window.akra.vaultRestore.projectChanged());
  await restore.locator('#root[data-phase="locked"]').waitFor({ timeout: 5_000 });
  check("a project switch locks the restore window", (await restore.getByTestId("vault-restore-detail").textContent())?.includes("프로젝트") === true);

  const protectedWindow = await app.evaluate(({ BrowserWindow }) =>
    BrowserWindow.getAllWindows().filter((window) => window.webContents.getURL().endsWith("vault-restore.html")).map((window) => window.isContentProtected?.() ?? null),
  );
  // Linux 에서는 OS 가 캡처 제외를 지원하지 않는다. 그때는 창이 하나인지만 본다.
  check("content protection is requested for the restore window", protectedWindow.length === 1 && (process.platform === "linux" || protectedWindow[0] !== false), JSON.stringify(protectedWindow));
} catch (error) {
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  const child = app.process();
  const exited = new Promise((resolve) => child.once("exit", resolve));
  for (const window of app.windows()) await window.close().catch(() => undefined);
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  if (child.exitCode === null) child.kill();
  await gateway.close();
  rmSync(userData, { recursive: true, force: true, maxRetries: 5 });
}
process.exit(failures.length === 0 ? 0 : 1);
