// 화면 시험 (plan 25): 빌드한 앱을 원격 호스트가 있는 가짜 Gateway 에 붙인다. 실제 클라우드와 턴은 쓰지 않는다.
// 원격 구역은 기본값이 꺼짐이고, 꺼져 있으면 원격 호스트에 접속하지 않는다. 켜면 그 호스트의 대화를 읽기 전용으로
// 보여 주고, 끄면 구역이 사라진다. 창은 숨긴 채로 돌린다.
import { mkdtempSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);

const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], { hosts: ["cloud"] });
// 켜고 끄는 설정을 사용자의 실제 앱 데이터 폴더에 쓰지 않는다.
const userData = mkdtempSync(join(tmpdir(), "akra-e2e-remote-"));
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;

const failures = [];
const check = (name, ok) => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}`);
  if (!ok) failures.push(name);
};

const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  const warnings = [];
  page.on("dialog", (dialog) => {
    warnings.push(dialog.message());
    void dialog.accept();
  });
  const toggleRemoteAgents = async () => {
    await page.getByRole("button", { name: "검색과 명령" }).click();
    await page.getByTestId("palette").getByText("클라우드 에이전트 켜기 / 끄기").click();
  };

  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().waitFor({ timeout: 15_000 });
  check("the remote section is hidden by default", (await page.getByTestId("remote-host").count()) === 0);
  check("no remote connection is opened while it is off", gateway.remoteReceivedMethods.length === 0);

  await toggleRemoteAgents();
  await page.getByTestId("remote-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 15_000 });
  check("turning it on shows a warning first", warnings.length === 1 && warnings[0].includes("약관"));
  await page.getByTestId("remote-thread-entry").first().click();
  await page.getByTestId("remote-banner").waitFor({ timeout: 10_000 });
  await page.getByTestId("turn").first().waitFor({ timeout: 15_000 });
  check("the remote thread is read over the remote connection", gateway.remoteReceivedMethods.includes("thread/resume") && !gateway.receivedMethods.includes("thread/resume"));
  check("the remote conversation has no composer", (await page.getByTestId("composer-input").count()) === 0);

  console.log("--- remote conversation text ---");
  console.log(await page.getByTestId("remote-banner").innerText());
  console.log((await page.getByTestId("remote-host").innerText()).split("\n").slice(0, 4).join(" | "));
  console.log("----------------------------");

  await toggleRemoteAgents();
  await page.getByTestId("remote-host").waitFor({ state: "detached", timeout: 10_000 });
  check("turning it off hides the section and leaves the remote conversation", (await page.getByTestId("remote-banner").count()) === 0);
  check("no turn, thread start, or kernel request went to the remote host", !gateway.remoteReceivedMethods.some((method) => method.startsWith("turn/") || method === "thread/start" || method.startsWith("akra/kernel/")));
} catch (error) {
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  // 창을 닫으면 앱이 끝난다. app.close() 는 쓰지 않는다 (P2 기록 참고).
  const exited = new Promise((resolve) => app.process().once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  await gateway.close();
  rmSync(userData, { recursive: true, force: true, maxRetries: 5 });
}
process.exit(failures.length === 0 ? 0 : 1);
