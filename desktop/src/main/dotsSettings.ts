// dots 클라우드 호스트의 활성화 플래그이다 (plan 25 §3.2, D-19). Gateway 는 시작할 때
// `$AKRA_HOME/dots/cloud-enabled` 가 있으면 `cloud` 호스트를 켠다 (production.rs `dots_cloud_flag_in`).
// Desktop 은 이 파일을 만들고 지운다. Gateway 는 시작 때 한 번만 읽으므로, 바꾼 뒤에는 Gateway 를 다시
// 시작해야 적용된다. 경로 규칙은 Rust 쪽과 같아야 한다.
import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

/** `$AKRA_HOME/dots/cloud-enabled`. `AKRA_HOME` 이 없으면 `~/.akra` 이다 (Gateway 의 akra_home_dir 과 같다). */
export function dotsCloudFlagPath(env: NodeJS.ProcessEnv = process.env, home: () => string = homedir): string {
  const akraHome = env.AKRA_HOME && env.AKRA_HOME.length > 0 ? env.AKRA_HOME : join(home(), ".akra");
  return join(akraHome, "dots", "cloud-enabled");
}

export function isDotsCloudEnabled(path: string = dotsCloudFlagPath()): boolean {
  return existsSync(path);
}

/** 플래그를 켜거나 끈다. 켜면 파일을 만들고, 끄면 지운다. 이미 그 상태면 아무것도 하지 않는다. */
export function setDotsCloudEnabled(enabled: boolean, path: string = dotsCloudFlagPath()): void {
  if (enabled) {
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, "1");
  } else {
    rmSync(path, { force: true });
  }
}
