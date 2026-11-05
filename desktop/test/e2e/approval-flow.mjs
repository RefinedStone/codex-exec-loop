// 화면 시험 (P3 §8): 빌드한 앱을 가짜 Gateway 에 붙여 승인 흐름을 재생한다. 실제 codex 와 턴은 쓰지 않는다.
// 사이드바 → 스레드 열기 → replay 된 승인 요청 → 허용 → 녹화된 턴이 끝까지 스트리밍 → 입력창이 새 턴 상태.
// Electron 실행 파일과 화면이 필요하므로 CI 가 아니라 로컬에서 `npm run e2e` 로 실행한다.
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);

const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.APPROVAL_TURN_RECORDING]);
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint };
delete env.ELECTRON_RUN_AS_NODE;

const failures = [];
const check = (name, ok) => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}`);
  if (!ok) failures.push(name);
};

const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("approval").waitFor({ timeout: 15_000 });
  check("replayed approval shows in the thread and the sidebar", (await page.getByTestId("approval-badge").count()) === 1);
  check("composer offers interrupt while the turn runs", (await page.getByTestId("composer-interrupt").count()) === 1);

  await page.getByTestId("approval-accept").click();
  await page.locator('[data-testid="turn"][data-status="completed"]').last().waitFor({ timeout: 15_000 });
  check("allowing removes the prompt and the recorded turn completes", (await page.getByTestId("approval").count()) === 0);
  check(
    "agent text and the command card render",
    (await page.getByTestId("item-agent").count()) > 0 && (await page.getByTestId("item-command").count()) > 0,
  );
  check("composer is ready for a new turn", (await page.getByTestId("composer-send").textContent()) === "보내기");
} catch (error) {
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  // 창을 닫으면 앱이 끝난다. app.close() 는 쓰지 않는다 (P2 기록 참고).
  const exited = new Promise((resolve) => app.process().once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  await gateway.close();
}
process.exit(failures.length === 0 ? 0 : 1);
