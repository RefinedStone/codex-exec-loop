// 앱 설정 파일이다 (plan 26 U2). userData 의 `app-settings.json` 에 둔다.
import { readFileSync, renameSync, writeFileSync } from "node:fs";

import { type AppSettings, validateSettings } from "../shared/settings";

export function loadAppSettings(path: string): AppSettings {
  try {
    return validateSettings(JSON.parse(readFileSync(path, "utf8")));
  } catch {
    return validateSettings(null);
  }
}

/** 임시 파일에 쓰고 바꿔 넣는다. 쓰지 못해도 앱은 계속 동작한다: 이 실행 동안에는 메모리의 값을 쓴다. */
export function saveAppSettings(path: string, settings: AppSettings): void {
  try {
    const temporary = `${path}.tmp`;
    writeFileSync(temporary, `${JSON.stringify(settings, null, 2)}\n`, "utf8");
    renameSync(temporary, path);
  } catch {
    // 저장하지 못했다.
  }
}
