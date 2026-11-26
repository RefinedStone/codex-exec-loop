import { existsSync, readdirSync, renameSync, rmdirSync } from "node:fs";
import { dirname, join } from "node:path";

/** userData 폴더 이름이 package 이름에서 제품명으로 바뀌기 전의 위치다 (#2588). */
export const LEGACY_USER_DATA_SEGMENTS = ["@refinedstone", "akra-desktop"] as const;
/** package.json 의 productName 과 같은 새 userData 폴더 이름이다. */
export const USER_DATA_DIRECTORY = "Akra ADE";
/** productName 을 정하기 전의 Electron 앱 이름(package.json name)이다. OS 키 저장소 항목 이름의 바탕이다. */
export const CREDENTIAL_APP_NAME = "@refinedstone/akra-desktop";

type NamedApp = Pick<Electron.App, "setName" | "whenReady">;

/**
 * safeStorage 의 OS 키 저장소 항목을 productName 변경 전과 같게 둔다 (#2588).
 * Electron 은 main script 를 실행한 뒤, ready 전에 앱 이름으로 그 항목을 고른다 (macOS `<이름> Safe Storage`,
 * Linux libsecret 의 application). 이름이 바뀌면 새 키가 생기고, 기존 암호문(이전 Jira·Linear 토큰, 브라우저 쿠키)을
 * 풀 수 없다. 그래서 ready 전에는 이전 이름을 쓰고, ready 뒤에 표시 이름을 제품명으로 돌린다.
 * main script 의 처음에, userData 를 정하기 전에 불러야 한다.
 */
export function keepCredentialIdentity(app: NamedApp): Promise<void> {
  app.setName(CREDENTIAL_APP_NAME);
  return app.whenReady().then(() => app.setName(USER_DATA_DIRECTORY));
}

export type UserDataMigration =
  | { kind: "none" }
  | { kind: "moved"; from: string; to: string }
  | { kind: "kept-legacy"; path: string; reason: string }
  | { kind: "both-exist"; legacy: string; current: string };

type UserDataApp = Pick<Electron.App, "getPath" | "setPath">;
type Fs = Pick<typeof import("node:fs"), "existsSync" | "readdirSync" | "renameSync" | "rmdirSync">;
const realFs: Fs = { existsSync, readdirSync, renameSync, rmdirSync };

/**
 * 이전 userData 폴더를 새 위치로 한 번 옮긴다. 새 폴더가 이미 있으면 아무것도 덮어쓰지 않는다.
 * 옮기지 못하면(이전 버전이 파일을 잡고 있는 경우 등) 이전 폴더를 계속 쓰고 다음 시작 때 다시 시도한다.
 * 이 함수는 app ready 전에, userData 를 읽는 어떤 코드보다 먼저 불러야 한다.
 */
export function migrateUserData(appData: string, current: string, fs: Fs = realFs): UserDataMigration {
  const legacy = join(appData, ...LEGACY_USER_DATA_SEGMENTS);
  if (legacy === current || !fs.existsSync(legacy)) return { kind: "none" };
  if (fs.existsSync(current)) return { kind: "both-exist", legacy, current };
  try {
    fs.renameSync(legacy, current);
  } catch (error) {
    return { kind: "kept-legacy", path: legacy, reason: error instanceof Error ? error.message : String(error) };
  }
  const scope = dirname(legacy);
  try {
    if (fs.readdirSync(scope).length === 0) fs.rmdirSync(scope);
  } catch {
    // 빈 scope 폴더가 남는 것은 데이터에 영향이 없다.
  }
  return { kind: "moved", from: legacy, to: current };
}

/**
 * Electron 의 userData 경로를 정한다. app ready 전에, userData 를 읽는 어떤 코드보다 먼저 불러야 한다.
 * `getPath("userData")` 는 폴더를 만들므로 이전을 정하기 전에는 부르지 않는다. 대상은 appData 에서 계산한다.
 */
export function prepareUserData(app: UserDataApp, override: string | undefined, fs: Fs = realFs): UserDataMigration {
  if (override) {
    app.setPath("userData", override);
    return { kind: "none" };
  }
  const appData = app.getPath("appData");
  const current = join(appData, USER_DATA_DIRECTORY);
  const migration = migrateUserData(appData, current, fs);
  app.setPath("userData", migration.kind === "kept-legacy" ? migration.path : current);
  return migration;
}
