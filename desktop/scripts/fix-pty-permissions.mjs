// node-pty 1.1.0 의 macOS prebuild 는 `spawn-helper` 에 실행 권한이 없다 (패키지 안의 모드가 0644).
// 그대로 두면 macOS 에서 `pty.spawn()` 이 `posix_spawnp failed` 로 실패한다 (microsoft/node-pty#919).
// 설치 뒤(postinstall)에 실행 권한을 되살린다.
// shortcut: 실행 권한이 고쳐진 stable node-pty 가 나오면 이 스크립트를 지운다. R8 패키징에서도 같은 권한이 필요하다.
import { chmodSync, existsSync, readdirSync, realpathSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/** prebuilds 아래의 모든 spawn-helper 에 실행 권한을 준다. 고친 파일 경로를 돌려준다. */
export function restoreSpawnHelperMode(nodePtyRoot) {
  const prebuilds = join(nodePtyRoot, "prebuilds");
  if (process.platform === "win32" || !existsSync(prebuilds)) {
    return [];
  }
  const fixed = [];
  for (const platform of readdirSync(prebuilds)) {
    const helper = join(prebuilds, platform, "spawn-helper");
    if (existsSync(helper) && (statSync(helper).mode & 0o111) === 0) {
      chmodSync(helper, 0o755);
      fixed.push(helper);
    }
  }
  return fixed;
}

// ESM 의 import.meta.url 은 symlink 를 푼 경로이고 argv[1] 은 아니다. 둘 다 풀어서 비교한다 (예: macOS /tmp).
const entry = process.argv[1];
if (entry && existsSync(entry) && fileURLToPath(import.meta.url) === realpathSync(entry)) {
  const root = join(dirname(fileURLToPath(import.meta.url)), "..", "node_modules", "node-pty");
  for (const helper of restoreSpawnHelperMode(root)) {
    console.log(`restored execute permission: ${helper}`);
  }
}
