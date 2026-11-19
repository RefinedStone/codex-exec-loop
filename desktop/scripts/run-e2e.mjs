// `npm run e2e` 의 실행기. test/e2e 의 모든 .mjs 를 차례로 실행해서 새 시험이 기본 gate 에서 빠지지 않게 한다.
import { spawnSync } from "node:child_process";
import { readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { defaultE2eSpecs } from "./e2e-specs.mjs";

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const e2eDir = join(desktopRoot, "test", "e2e");
const specs = defaultE2eSpecs(readdirSync(e2eDir));
if (specs.length === 0) {
  // 시험을 하나도 찾지 못하면 gate 가 조용히 통과하지 않게 한다.
  console.error(`e2e: no specs found in ${e2eDir}`);
  process.exit(1);
}
for (const spec of specs) {
  console.log(`e2e: ${spec}`);
  // 시험은 이 스크립트와 같은 node 옵션(--experimental-strip-types 등)으로 실행한다.
  const result = spawnSync(process.execPath, [...process.execArgv, join(e2eDir, spec)], { cwd: desktopRoot, stdio: "inherit" });
  if (result.status !== 0) {
    console.error(`e2e: ${spec} failed (${result.error?.message ?? result.signal ?? `exit ${result.status}`})`);
    process.exit(result.status || 1);
  }
}
