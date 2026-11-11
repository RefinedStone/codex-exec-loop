// `npm run blueprint`: UX 블루프린트 화면을 연다. mock 데이터만 쓰고 Gateway 와 codex 를 쓰지 않는다.
// `npm run dev` 와 같은 길로 열고 (Electron 바이너리 확인 포함), 환경 변수로 블루프린트 화면을 고른다.
import { spawnSync } from "node:child_process";

const windows = process.platform === "win32";
const result = spawnSync(windows ? "npm.cmd" : "npm", ["run", "dev"], {
  stdio: "inherit",
  env: { ...process.env, AKRA_DESKTOP_BLUEPRINT: "1" },
  shell: windows,
});
process.exit(result.status ?? 1);
