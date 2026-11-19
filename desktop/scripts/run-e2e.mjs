// `npm run e2e` 의 화면 시험 목록. test/e2e 의 모든 .mjs 를 차례로 실행해서 새 시험이 기본 gate 에서 빠지지 않게 한다.
import { spawnSync } from "node:child_process";
import { readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** 기본 gate 에서 빼는 수동 시험과 그 이유. */
export const MANUAL_E2E_SPECS = new Map([
  ["browserAutomation.mjs", "opens a visible Electron window (show: true)"],
  ["browserBridge.mjs", "needs AKRA_GATEWAY_BIN to point to a built native gateway"],
]);

/** e2e 폴더의 파일 이름에서 기본 gate 가 실행할 시험을 이름 순서로 돌려준다. */
export function defaultE2eSpecs(fileNames) {
  return fileNames.filter((name) => name.endsWith(".mjs") && !MANUAL_E2E_SPECS.has(name)).sort();
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const e2eDir = join(desktopRoot, "test", "e2e");
  for (const spec of defaultE2eSpecs(readdirSync(e2eDir))) {
    console.log(`e2e: ${spec}`);
    // 시험은 이 스크립트와 같은 node 옵션(--experimental-strip-types 등)으로 실행한다.
    const result = spawnSync(process.execPath, [...process.execArgv, join(e2eDir, spec)], { cwd: desktopRoot, stdio: "inherit" });
    if (result.status !== 0) {
      console.error(`e2e: ${spec} failed (${result.error?.message ?? result.signal ?? `exit ${result.status}`})`);
      process.exit(result.status || 1);
    }
  }
}
