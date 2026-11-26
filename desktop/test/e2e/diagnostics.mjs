// A10: 실제 Electron/preload/main 경계의 오프라인 수집→검토→취소/저장 실패/동일 바이트 저장. 모델 호출 없음.
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { _electron as electron } from "playwright-core";

const desktopDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const root = mkdtempSync(join(tmpdir(), "akra-e2e-diagnostic-"));
const canary = "PRIVATE_DIAGNOSTIC_CANARY";
const logPath = join(root, "trace.jsonl");
writeFileSync(logPath, JSON.stringify({ target: "akra_ade::adapter::inbound::gateway::session", level: "WARN", timestamp: "2026-11-26T01:02:03Z", message: canary, error: canary, path: root, token: canary }) + "\n");
const endpoint = process.platform === "win32" ? `\\\\.\\pipe\\akra-diagnostic-missing-${process.pid}` : join(root, "missing.sock");
const env = { ...process.env, AKRA_DESKTOP_USER_DATA: join(root, "user-data"), AKRA_DESKTOP_HIDDEN: "1", AKRA_GATEWAY_ENDPOINT: endpoint, AKRA_GATEWAY_BIN: join(root, "no-daemon"), AKRA_TRACE_FILE: logPath };
delete env.ELECTRON_RUN_AS_NODE;
const app = await electron.launch({ executablePath: createRequire(import.meta.url)("electron"), args: [desktopDir, ...(process.env.CI ? ["--no-sandbox"] : [])], env });
try {
  const page = await app.firstWindow();
  await page.getByTestId("connection-phase").filter({ hasText: "끊김" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("diagnostics-open").click();
  await page.getByTestId("diagnostics-content").waitFor();
  const content = await page.getByTestId("diagnostics-content").inputValue();
  assert.ok(!content.includes(canary) && !content.includes(root));
  assert.ok((await page.getByTestId("diagnostics-status").textContent()).includes("오프라인"));
  assert.equal(JSON.parse(content).logs.entries.length, 1);
  // GatewayConnection은 errno.code를 전달한다. 자유형 메시지를 export에서 파싱하지 않는다.
  assert.equal(JSON.parse(content).connection.errorCode, "ENOENT");
  assert.ok(!content.includes(endpoint));
  await app.evaluate(({ dialog }) => { dialog.showSaveDialog = async () => ({ canceled: true, filePath: undefined }); });
  await page.getByTestId("diagnostics-save").click();
  await page.getByTestId("diagnostics-result").filter({ hasText: "취소" }).waitFor();
  await app.evaluate(({ dialog }, path) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath: path }); }, join(root, "absent-directory", "failed.json"));
  await page.getByTestId("diagnostics-save").click();
  await page.getByTestId("diagnostics-result").filter({ hasText: "저장하지 못했습니다" }).waitFor();
  const destination = join(root, "reviewed.json");
  await app.evaluate(({ dialog }, path) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath: path }); }, destination);
  await page.getByTestId("diagnostics-save").click();
  await page.getByTestId("diagnostics-result").filter({ hasText: "저장했습니다" }).waitFor();
  assert.equal(readFileSync(destination, "utf8"), content);
  // 디스크의 원본은 저장 직전에 다시 수집하지 않는다.
  writeFileSync(logPath, "");
  await page.getByTestId("diagnostics-save").click();
  await page.getByTestId("diagnostics-result").filter({ hasText: "저장했습니다" }).waitFor();
  assert.equal(readFileSync(destination, "utf8"), content);
  await page.getByRole("button", { name: "닫기", exact: true }).click();
  const stale = await page.evaluate(id => window.akra.diagnostics.save(id), JSON.parse(content).diagnosticId);
  assert.equal(stale.status, "failed");
  assert.equal(stale.reason, "expired");
  if (process.env.AKRA_E2E_ARTIFACT_DIR) {
    await page.getByTestId("diagnostics-open").click();
    await page.getByTestId("diagnostics-content").waitFor();
    await page.screenshot({ path: join(process.env.AKRA_E2E_ARTIFACT_DIR, "offline-diagnostic.png") });
  }
  console.log("PASS local diagnostic preview, redaction, cancel, failure, exact save and expiration");
} finally {
  await app.close();
  rmSync(root, { recursive: true, force: true, maxRetries: 5 });
}
