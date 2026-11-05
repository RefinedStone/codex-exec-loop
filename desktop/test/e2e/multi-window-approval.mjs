// 화면 시험 (P3 §8, 여러 창): 창 두 개가 같은 스레드를 열고 replay 된 승인 요청을 함께 받는다. 한 창이 허용하면
// 다른 창의 승인 표시는 `serverRequest/resolved` 로 사라지고, 가짜 Gateway 는 먼저 온 답 하나만 쓴다.
// 빌드한 앱과 가짜 Gateway 를 쓴다. 실제 codex 와 턴은 쓰지 않는다. 로컬에서 `npm run e2e` 로 실행한다.
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
  const first = await app.firstWindow();
  const secondPromise = app.waitForEvent("window");
  await app.evaluate(({ Menu }) => Menu.getApplicationMenu()?.getMenuItemById("new-window")?.click());
  const second = await secondPromise;
  check("the menu opens a second window", app.windows().length === 2);

  for (const page of [first, second]) {
    await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
    await page.getByTestId("thread-entry").first().click();
    await page.getByTestId("approval").waitFor({ timeout: 15_000 });
  }
  check("both windows show the replayed approval", true);

  await first.getByTestId("approval-accept").click();
  await second.getByTestId("approval").waitFor({ state: "detached", timeout: 15_000 });
  check("answering in one window removes the prompt in the other", (await second.getByTestId("approval").count()) === 0);
  await first.locator('[data-testid="turn"][data-status="completed"]').last().waitFor({ timeout: 15_000 });
  check("the answering window streams the turn to completion", true);
  check(
    "the gateway used exactly one answer, the first one",
    gateway.answeredRequests.length === 1 && gateway.answeredRequests[0].result?.decision === "accept",
  );
} catch (error) {
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  const exited = new Promise((resolve) => app.process().once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  await gateway.close();
}
process.exit(failures.length === 0 ? 0 : 1);
