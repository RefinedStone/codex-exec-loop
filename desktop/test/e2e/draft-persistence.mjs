// 화면 시험 (#2839): 스레드의 초안이 앱을 다시 열어도 남고, 대화 영역에서 친 글자는 입력창으로 간다.
// 실제 codex 와 턴은 쓰지 않는다 (가짜 Gateway). userData 는 임시 폴더를 쓴다.
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);

const failures = [];
const check = (name, ok, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${!ok && detail ? ` | ${detail}` : ""}`);
  if (!ok) failures.push(name);
};

const userData = mkdtempSync(join(tmpdir(), "akra-e2e-drafts-"));
const endpoint = fake.uniqueTestEndpoint();
const predictions = join(userData, "stale-prediction.jsonl");
const threadId = "01a1abfb-8f0a-7581-a8e4-c2953347f470";
const method = "thread/prediction/updated";
writeFileSync(predictions, [
  ["client", { method: "fixture/prediction" }],
  ["gateway", { method, params: { threadId, sourceTurnId: "old", result: { type: "completed", text: "보내면 안 되는 제안" } } }],
  // Ordered barrier: the following name update proves the preceding late notification arrived.
  ["gateway", { method: "thread/name/updated", params: { threadId, threadName: "알림 수신 확인" } }],
].map(([direction, frame]) => JSON.stringify({ direction, text: JSON.stringify(frame) })).join("\n"));
const handshakes = [];
const gateway = await fake.startFakeGateway(endpoint, [predictions, fake.APPROVAL_THREAD_LIST_RECORDING, fake.APPROVAL_TURN_RECORDING], {
  onRequest(name, params) { if (name === "initialize") handshakes.push(params.capabilities); },
});
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;

async function withApp(run) {
  const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
  try {
    const page = await app.firstWindow();
    await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
    await page.getByTestId("thread-entry").first().click();
    await page.getByTestId("conversation").waitFor({ timeout: 15_000 });
    await run(page);
  } finally {
    const exited = new Promise((resolve) => app.process().once("exit", resolve));
    for (const window of app.windows()) await window.close();
    await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  }
}

try {
  await withApp(async (page) => {
    const input = page.getByTestId("composer-input");
    check("Desktop requests suppression on the stable connection", handshakes.at(-1)?.experimentalApi === false
      && handshakes.at(-1)?.optOutNotificationMethods?.includes(method));
    await page.evaluate(() => window.akra.send(JSON.stringify({ method: "fixture/prediction" })));
    await page.getByTestId("thread-entry").filter({ hasText: "알림 수신 확인" }).waitFor();
    await input.press("Tab");
    await input.press("Enter");
    check("an unsolicited prediction neither fills nor sends an empty draft", await input.inputValue() === ""
      && !gateway.receivedMethods.some(name => ["thread/prediction/request", "thread/sample", "turn/start", "turn/steer"].includes(name)));
    await input.fill("아직 보내지 않은 초안");
    // 입력창 밖(대화 영역)을 누르고 글자를 치면 입력창으로 간다.
    await page.getByTestId("conversation").click({ position: { x: 5, y: 5 } });
    check("clicking the conversation moves focus out of the composer", !(await input.evaluate((element) => element === document.activeElement)));
    await page.keyboard.type("!");
    check("a letter typed in the conversation goes to the composer", (await input.inputValue()) === "아직 보내지 않은 초안!", await input.inputValue());
    // 찾기 칸의 글자는 그 칸의 것이다.
    await page.keyboard.press("ControlOrMeta+KeyF");
    await page.getByTestId("find-input").fill("abc");
    check("typing in the find bar stays in the find bar", (await input.inputValue()) === "아직 보내지 않은 초안!", await input.inputValue());
    await page.keyboard.press("Escape");
  });
  await withApp(async (page) => {
    const input = page.getByTestId("composer-input");
    check("the thread draft is back after the app restarts", (await input.inputValue()) === "아직 보내지 않은 초안!", await input.inputValue());
    // 저장 완료를 기다리는 polling 없이 마지막 입력 직후 창을 닫는다.
    await input.fill("종료 직전의 마지막 초안");
  });
  await withApp(async (page) => {
    const input = page.getByTestId("composer-input");
    check("the last edit survives closing immediately after typing", (await input.inputValue()) === "종료 직전의 마지막 초안", await input.inputValue());
  });
} catch (error) {
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  await gateway.close();
  rmSync(userData, { recursive: true, force: true, maxRetries: 5 });
}
process.exit(failures.length === 0 ? 0 : 1);
