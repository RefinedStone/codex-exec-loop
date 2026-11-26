// 화면 시험 (#2838): 입력창에 붙여넣은 이미지가 미리보기로 보이고, 보내면 turn/start 입력의 image 항목으로 간다.
// 이미지가 아닌 파일과 너무 큰 이미지는 이유와 함께 거절한다. 실제 codex 와 턴은 쓰지 않는다 (가짜 Gateway).
import { mkdtempSync, rmSync } from "node:fs";
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

const starts = [];
const userData = mkdtempSync(join(tmpdir(), "akra-e2e-image-attach-"));
const endpoint = fake.uniqueTestEndpoint();
const gateway = await fake.startFakeGateway(endpoint, [fake.APPROVAL_THREAD_LIST_RECORDING, fake.INTERRUPTED_TURN_RECORDING], {
  onRequest(method, params) {
    if (method === "turn/start" || method === "turn/steer") starts.push(params);
  },
});
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;

const paste = (page, files) => page.getByTestId("composer-input").evaluate((element, list) => {
  const data = new DataTransfer();
  for (const { name, type, size } of list) data.items.add(new File([new Uint8Array(size)], name, { type }));
  element.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }));
}, files);

const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("composer-input").waitFor({ timeout: 15_000 });

  await paste(page, [{ name: "shot.png", type: "image/png", size: 64 }]);
  await page.getByTestId("attachments").waitFor({ timeout: 5_000 });
  check("a pasted image shows a preview", (await page.locator('[data-testid="attachments"] img').count()) === 1);
  await paste(page, [{ name: "huge.png", type: "image/png", size: 10 * 1024 * 1024 + 1 }]);
  await page.locator(".composer-area .error").filter({ hasText: "MB 까지" }).waitFor({ timeout: 5_000 });
  check("an image over the size limit is refused with a reason", (await page.locator('[data-testid="attachments"] img').count()) === 1);

  await page.getByTestId("composer-input").fill("이 화면을 보세요");
  await page.getByTestId("composer-send").click();
  const deadline = Date.now() + 10_000;
  while (starts.length === 0 && Date.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 50));
  const input = starts[0]?.input ?? [];
  check("the image goes to the turn input after the text", input.length === 2 && input[0].type === "text"
    && input[1].type === "image" && String(input[1].url).startsWith("data:image/png;base64,"), JSON.stringify(input).slice(0, 200));
  await page.waitForFunction(() => document.querySelector('[data-testid="attachments"]') === null, null, { timeout: 5_000 });
  check("the attachments clear after sending", true);
} catch (error) {
  check(`flow finished without error (${String(error).split("\n")[0]})`, false);
} finally {
  const exited = new Promise((resolve) => app.process().once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  await gateway.close();
  rmSync(userData, { recursive: true, force: true, maxRetries: 5 });
}
process.exit(failures.length === 0 ? 0 : 1);
