// 화면 시험 (#2427): 로컬 복원본 창을 열고, 자동화로 실행한 앱에서는 열람을 보류하는지, 그 창이 일반 표면과 나뉘어
// 있는지 확인한다. Playwright 프로필에서는 실제 native 복원이 차단된다. 별도로 main 시험 harness가 보낸
// synthetic restored state로 전용 화면의 평문 표시/삭제만 확인하며, native 승인 성공으로 취급하지 않는다.
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
const TARGET = { workspace: desktopDir, issueUrl: ISSUE, issue: { provider: "github", server_id: "github.com", repository_id: "R_fixture", issue_id: "I_fixture", project_scope: { workspace_dir: desktopDir } } };

const failures = [];
const check = (name, ok, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${!ok && detail ? ` | ${detail}` : ""}`);
  if (!ok) failures.push(name);
};

const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });

  // 주소만 보내거나 workspace·전체 좌표가 맞지 않으면 일반 창에서 열 수 없다.
  for (const target of [ISSUE, { ...TARGET, issueUrl: "https://example.test/not-an-issue" }, { ...TARGET, workspace: desktopDir + "/other" }, { ...TARGET, approved: true }]) {
    const refused = await page.evaluate((target) => window.akra.vaultRestore.open(target).then(() => false, () => true), target);
    check("an incomplete, mismatched or untrusted target is refused", refused);
  }

  const opened = app.waitForEvent("window");
  await page.evaluate((target) => window.akra.vaultRestore.open(target), TARGET);
  const restore = await opened;
  await restore.locator('#root[data-phase="held"]').waitFor({ timeout: 10_000 });
  check("the restore window opens with its own page", restore.url().endsWith("vault-restore.html"), restore.url());
  check("viewing is held under automation", (await restore.getByTestId("vault-restore-detail").textContent())?.includes("자동화") === true);
  check("no restored text is on the screen", (await restore.getByTestId("vault-restored-body").count()) === 0);

  // 그 창에는 일반 메시지 버스가 없고, 일반 창에는 복원본 채널이 없다.
  check("the restore window has no general bridge", await restore.evaluate(() => typeof window.akra === "object" && typeof window.akra.send === "undefined" && typeof window.akra.vaultRestore.onState === "function"));
  check("the general window has no restore bridge", await page.evaluate(() => typeof window.akra.vaultRestore.onState === "undefined" && typeof window.akraVaultRestore === "undefined"));

  // 같은 이슈를 다시 열면 창을 하나 더 만들지 않는다.
  await page.evaluate((target) => window.akra.vaultRestore.open(target), TARGET);
  check("the same issue reuses its window", app.windows().length === 2, String(app.windows().length));

  // 다시 여는 요청도 실행 프로필을 다시 본다.
  await restore.getByTestId("vault-restore-unlock").click();
  await restore.locator('#root[data-phase="held"]').waitFor({ timeout: 5_000 });
  check("another unlock is held again", (await restore.getByTestId("vault-restore-heading").textContent()) === "열람 보류");

  // 프로젝트 전환 알림은 그 창을 잠근다.
  await page.evaluate(() => window.akra.vaultRestore.projectChanged());
  await restore.locator('#root[data-phase="locked"]').waitFor({ timeout: 5_000 });
  check("a project switch locks the restore window", (await restore.getByTestId("vault-restore-detail").textContent())?.includes("프로젝트") === true);

  // 같은 주소여도 전체 좌표가 다르면 승인 대상과 창을 합치지 않는다.
  const anotherOpened = app.waitForEvent("window");
  await page.evaluate((target) => window.akra.vaultRestore.open(target), { ...TARGET, issue: { ...TARGET.issue, issue_id: "I_another" } });
  const anotherRestore = await anotherOpened;
  await anotherRestore.locator('#root[data-phase="held"]').waitFor({ timeout: 5_000 });
  check("different full coordinates do not reuse a URL-only window", app.windows().length === 3);
  await anotherRestore.close();

  // 이전 프로젝트의 창에서 직접 unlock해도 새 helper를 시작할 수 없다.
  await restore.getByTestId("vault-restore-unlock").click();
  await restore.locator('#root[data-phase="held"]').waitFor({ timeout: 5_000 });
  check("a switched-away target cannot unlock itself", (await restore.getByTestId("vault-restore-detail").textContent())?.includes("복원 경로") === true);
  await page.evaluate((target) => window.akra.vaultRestore.open(target), TARGET);
  await restore.getByTestId("vault-restore-unlock").click();
  await restore.locator('#root[data-phase="held"]').waitFor({ timeout: 5_000 });
  check("a fresh current-target open still obeys the automation hold", (await restore.getByTestId("vault-restore-detail").textContent())?.includes("자동화") === true);

  // 화면만 검증하는 합성 fixture이다. 제품에 승인 우회·fake environment·helper 대체 경로는 없다.
  const synthetic = { title: "synthetic local title <b>literal</b>", body: "synthetic-private-body <img src=x onerror=alert(1)> **literal**" };
  const showSynthetic = async (state) => app.evaluate(({ BrowserWindow }, state) => {
    const window = BrowserWindow.getAllWindows().find((owner) => owner.webContents.getURL().endsWith("vault-restore.html"));
    window.webContents.send("akra-vault-restore:state", state);
  }, state);
  await showSynthetic({ phase: "restored", ...synthetic, expiresAt: Date.now() + 60_000 });
  await restore.getByTestId("vault-restored-body").waitFor();
  check("synthetic restored state appears only as literal text", await restore.getByTestId("vault-restored-title").textContent() === synthetic.title
    && await restore.getByTestId("vault-restored-body").textContent() === synthetic.body
    && await restore.locator("#root img, #root b").count() === 0);
  check("the general window never receives synthetic plaintext", !(await page.locator("body").textContent()).includes("synthetic-private-body"));
  await restore.getByTestId("vault-restore-lock").click();
  await restore.locator('#root[data-phase="locked"]').waitFor();
  check("locking removes synthetic plaintext from the dedicated DOM", !(await restore.locator("body").textContent()).includes("synthetic-private-body"));
  await showSynthetic({ phase: "held", reason: "marker_moved" });
  await restore.locator('#root[data-phase="held"]').waitFor();
  check("unsupported reconnect is absent from the dedicated screen", await restore.getByTestId("vault-restore-review").count() === 0
    && (await restore.getByTestId("vault-restore-detail").textContent()).includes("재연결을 지원하지 않습니다"));

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
