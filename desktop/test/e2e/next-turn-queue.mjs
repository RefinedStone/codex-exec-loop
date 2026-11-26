// Real Electron UI, synthetic Gateway only: reserving input never calls Codex.
import { mkdtempSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const electronPath = createRequire(import.meta.url)("electron");
const fake = await import(pathToFileURL(join(desktopDir, "test/fake-gateway/fakeGateway.ts")).href);
const requests = [];
const gateway = await fake.startFakeGateway(fake.uniqueTestEndpoint(), [fake.APPROVAL_THREAD_LIST_RECORDING, fake.APPROVAL_TURN_RECORDING], {
  appServer(method, params) {
    if (method.startsWith("turn/")) requests.push({ method, params });
    if (method === "turn/start") return { result: { turn: { id: "queued-turn", status: "inProgress", items: [], error: null } } };
    return undefined;
  },
});
const userData = mkdtempSync(join(tmpdir(), "akra-e2e-queue-"));
const env = { ...process.env, AKRA_GATEWAY_ENDPOINT: gateway.endpoint, AKRA_DESKTOP_HIDDEN: "1", AKRA_DESKTOP_USER_DATA: userData };
delete env.ELECTRON_RUN_AS_NODE;
const app = await electron.launch({ executablePath: electronPath, args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.getByTestId("connection-phase").filter({ hasText: "연결됨" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("thread-entry").first().click();
  await page.getByTestId("approval").waitFor({ timeout: 15_000 });
  await page.getByTestId("composer-input").fill("다음 작업의 첫 초안");
  await page.getByTestId("composer-queue").click();
  const queued = page.getByTestId("queued-inputs");
  await queued.waitFor();
  if (requests.length !== 0) throw new Error("reservation dispatched during the active turn");
  await queued.getByRole("button", { name: "수정", exact: true }).click();
  await queued.getByRole("textbox", { name: "예약 입력 수정" }).fill("수정한 다음 작업");
  await queued.getByRole("button", { name: "수정 저장", exact: true }).click();
  await page.getByTestId("composer-input").fill("삭제할 예약");
  await page.getByTestId("composer-queue").click();
  await queued.getByRole("button", { name: "삭제", exact: true }).last().click();
  if (!(await queued.textContent()).includes("이 창을 닫으면 지워집니다")) throw new Error("missing memory lifetime notice");
  if (process.env.AKRA_QA_SCREENSHOT_DIR) await page.screenshot({ path: join(process.env.AKRA_QA_SCREENSHOT_DIR, "next-turn-queue.png"), animations: "disabled" });
  await page.getByTestId("approval-accept").click();
  await page.locator('[data-testid="turn"][data-status="completed"]').last().waitFor({ timeout: 15_000 });
  await queued.waitFor({ state: "detached", timeout: 15_000 });
  if (requests.length !== 1 || requests[0].method !== "turn/start" || requests[0].params.input[0].text !== "수정한 다음 작업") {
    throw new Error("queued input did not dispatch once as a new turn: " + JSON.stringify(requests));
  }
  console.log("PASS reservation/edit/delete waits for completion and starts exactly one synthetic turn");
} finally {
  const exited = new Promise((resolve) => app.process().once("exit", resolve));
  for (const window of app.windows()) await window.close();
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 10_000))]);
  await gateway.close();
  rmSync(userData, { recursive: true, force: true, maxRetries: 5 });
}
