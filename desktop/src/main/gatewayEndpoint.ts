// Gateway 의 접속 주소와 실행 파일 위치를 정한다 (P-04).
// - unix: $AKRA_HOME/run/gateway.sock (기본 ~/.akra)
// - Windows: \\.\pipe\akra-gateway-<현재 사용자 SID>
import { execFileSync } from "node:child_process";
import { existsSync, statSync } from "node:fs";
import { join, posix, resolve, win32 } from "node:path";

export function resolveGatewayEndpoint(
  platform: NodeJS.Platform = process.platform,
  env: NodeJS.ProcessEnv = process.env,
  readUserSid: () => string = readWindowsUserSid,
): string {
  // 개발과 화면 시험에서 가짜 Gateway 에 붙일 때 쓴다.
  if (env.AKRA_GATEWAY_ENDPOINT && env.AKRA_GATEWAY_ENDPOINT.length > 0) {
    return env.AKRA_GATEWAY_ENDPOINT;
  }
  const akraHome = resolveAkraHome(platform, env);
  if (platform === "win32") {
    return `\\\\.\\pipe\\akra-gateway-${readUserSid()}`;
  }
  return join(akraHome, "run", "gateway.sock");
}

/** Match native configuration and planning: invalid explicit roots never fall back. */
export function resolveAkraHome(platform: NodeJS.Platform, env: NodeJS.ProcessEnv): string {
  const windows = platform === "win32";
  const paths = windows ? win32 : posix;
  const profile = windows ? "USERPROFILE" : "HOME";
  const explicit = env.AKRA_HOME;
  const root = explicit === undefined ? env[profile] : explicit;
  if (!root) throw new Error(explicit === undefined
    ? `AKRA_HOME is unset and ${profile} is unavailable; set an explicit absolute AKRA_HOME`
    : "AKRA_HOME must not be empty; unset it to use the default profile");
  // win32.isAbsolute also accepts a drive-relative root such as backslash-foo.
  const separated = windows ? root.replaceAll(String.fromCharCode(92), "/") : root;
  const fullyQualified = !windows || /^[a-z]:\//i.test(separated) || /^\/\/[^/]+\/[^/]+/.test(separated);
  if (!paths.isAbsolute(root) || !fullyQualified || separated.split("/").includes("..")) {
    throw new Error("AKRA_HOME must be an absolute normalized path");
  }
  return explicit === undefined ? paths.join(root, ".akra") : root;
}

/** `whoami /user /fo csv /nh` 출력(`"domain\user","S-1-5-21-..."`)에서 SID 를 꺼낸다. */
export function parseWhoamiSid(output: string): string {
  const match = /"(S-1-[0-9-]+)"/.exec(output);
  if (!match?.[1]) {
    throw new Error("could not read the current user SID from whoami output");
  }
  return match[1];
}

/** PATH 의 `whoami` 는 Git Bash 의 coreutils 일 수 있으므로 시스템 경로의 실행 파일을 쓴다. */
export function windowsWhoamiPath(env: NodeJS.ProcessEnv = process.env): string {
  return join(env.SystemRoot ?? env.SYSTEMROOT ?? "C:\\Windows", "System32", "whoami.exe");
}

function readWindowsUserSid(): string {
  const output = execFileSync(windowsWhoamiPath(), ["/user", "/fo", "csv", "/nh"], {
    encoding: "utf8",
    windowsHide: true,
  });
  return parseWhoamiSid(output);
}

/**
 * 데몬을 시작할 실행 파일이다. 순서: `AKRA_GATEWAY_BIN`, 저장소의 release·debug 빌드 중 더 최근 것, PATH 의
 * `akra-gateway`. `npm run dev` 는 debug 를 빌드하므로, 오래된 release 빌드가 새 debug 빌드를 가리면 안 된다.
 */
export function resolveGatewayExecutable(
  repositoryRoot: string,
  platform: NodeJS.Platform = process.platform,
  env: NodeJS.ProcessEnv = process.env,
  modifiedAt: (path: string) => number | null = readModifiedAt,
): string {
  if (env.AKRA_GATEWAY_BIN && env.AKRA_GATEWAY_BIN.length > 0) {
    return env.AKRA_GATEWAY_BIN;
  }
  const fileName = platform === "win32" ? "akra-gateway.exe" : "akra-gateway";
  let newest: { path: string; modifiedAt: number } | null = null;
  for (const profile of ["release", "debug"]) {
    const candidate = resolve(repositoryRoot, "target", profile, fileName);
    const time = modifiedAt(candidate);
    if (time !== null && (newest === null || time > newest.modifiedAt)) {
      newest = { path: candidate, modifiedAt: time };
    }
  }
  return newest?.path ?? fileName;
}

function readModifiedAt(path: string): number | null {
  return existsSync(path) ? statSync(path).mtimeMs : null;
}
