// 스킬 파일 묶음과 매니페스트 해시이다 (#2383). 설치 폴더와 내려받은 묶음에 같은 규칙을 쓴다.
//
// 매니페스트 해시 규칙 v1:
// - 대상은 스킬 폴더 아래의 모든 일반 파일이다 (`SKILL.md` 만이 아니다). `.DS_Store` 는 뺀다.
// - 경로는 스킬 폴더 기준이고 `/` 로 나눈다. 코드 단위 순서로 정렬한다.
// - 파일마다 `<경로>\0<내용의 sha256 hex>\n` 을 이어 붙여 sha256 을 구한다. 결과는 `v1:<hex>` 이다.
// - 내용은 바이트 그대로이다. 줄바꿈을 바꾸지 않는다: CRLF 로 바뀐 복사본은 다른 묶음이다.
// - 실행 권한은 넣지 않는다. Windows 에서 읽을 수 없다.
import { createHash } from "node:crypto";
import { lstatSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

export const MANIFEST_VERSION = 1;

/** shortcut: 고정 한도이다. 스킬은 작은 글 묶음이다. 더 큰 스킬이 나오면 설정으로 옮긴다. */
export const SKILL_LIMITS = { files: 500, fileBytes: 2 * 1024 * 1024, totalBytes: 16 * 1024 * 1024 };

const IGNORED_NAMES = new Set([".DS_Store"]);

export interface SkillFile {
  /** 스킬 폴더 기준 경로이다 (`/` 구분). */
  path: string;
  bytes: Buffer;
  executable: boolean;
}

/** 스킬 묶음으로 받을 수 없는 내용이다 (링크, 비정상 파일, 루트 밖 경로, 한도 초과). */
export class SkillBundleError extends Error {}

function sha256(data: string | Buffer): string {
  return createHash("sha256").update(data).digest("hex");
}

export function manifestHash(files: readonly Pick<SkillFile, "path" | "bytes">[]): string {
  const lines = files
    .map((file) => `${file.path}\0${sha256(file.bytes)}\n`)
    .sort((left, right) => (left < right ? -1 : left > right ? 1 : 0));
  return `v${MANIFEST_VERSION}:${sha256(lines.join(""))}`;
}

// Windows 가 파일 이름에 받지 않는 글자와 예약 이름이다. macOS 에서 받은 묶음도 Windows 에 쓸 수 있어야 한다.
const FORBIDDEN_CHARACTERS = /[\0-\x1f<>:"|?*\\]/;
const RESERVED_NAMES = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(\..*)?$/i;

/** 스킬 폴더 기준 경로를 검사한다. 받을 수 없으면 이유를 돌려준다. */
export function relativePathProblem(path: string): string | null {
  if (path.length === 0) return "빈 경로";
  if (path.startsWith("/")) return "절대 경로";
  for (const segment of path.split("/")) {
    if (segment === "" || segment === "." || segment === "..") return "루트 밖을 가리킬 수 있는 경로";
    if (FORBIDDEN_CHARACTERS.test(segment)) return "받을 수 없는 글자 (역슬래시, 드라이브, 제어 문자)";
    if (RESERVED_NAMES.test(segment) || /[. ]$/.test(segment)) return "Windows 가 받지 않는 이름";
    if (segment.toLowerCase() === ".git") return ".git 폴더";
  }
  return null;
}

/** 묶음 전체를 검사한다: 경로, 대소문자만 다른 중복, 파일 수와 크기 한도. */
export function assertBundle(files: readonly SkillFile[]): void {
  if (files.length > SKILL_LIMITS.files) throw new SkillBundleError(`파일이 너무 많습니다 (${files.length}개, 한도 ${SKILL_LIMITS.files}개).`);
  const seen = new Set<string>();
  let total = 0;
  for (const file of files) {
    const problem = relativePathProblem(file.path);
    if (problem) throw new SkillBundleError(`받을 수 없는 경로입니다 (${problem}): ${JSON.stringify(file.path)}`);
    // 대소문자를 구분하지 않는 파일시스템에서는 두 파일이 하나를 덮어쓴다.
    const folded = file.path.toLowerCase();
    if (seen.has(folded)) throw new SkillBundleError(`대소문자만 다른 경로가 겹칩니다: ${file.path}`);
    seen.add(folded);
    if (file.bytes.length > SKILL_LIMITS.fileBytes) throw new SkillBundleError(`파일이 너무 큽니다: ${file.path}`);
    total += file.bytes.length;
  }
  if (total > SKILL_LIMITS.totalBytes) throw new SkillBundleError("파일 묶음이 너무 큽니다.");
  if (!files.some((file) => file.path === "SKILL.md")) throw new SkillBundleError("SKILL.md 가 없습니다.");
}

/**
 * 폴더의 파일 묶음을 읽는다. 링크를 따라가지 않는다: 심볼릭 링크나 일반 파일이 아닌 것(소켓, 장치)이 있으면
 * 묶음 전체를 거절한다.
 */
export function readSkillFiles(directory: string): SkillFile[] {
  const files: SkillFile[] = [];
  const walk = (absolute: string, relative: string): void => {
    for (const name of readdirSync(absolute).sort()) {
      if (IGNORED_NAMES.has(name)) continue;
      const path = relative ? `${relative}/${name}` : name;
      const target = join(absolute, name);
      const stat = lstatSync(target);
      if (stat.isSymbolicLink()) throw new SkillBundleError(`심볼릭 링크가 있습니다: ${path}`);
      if (stat.isDirectory()) {
        walk(target, path);
      } else if (stat.isFile()) {
        if (stat.size > SKILL_LIMITS.fileBytes) throw new SkillBundleError(`파일이 너무 큽니다: ${path}`);
        if (files.length >= SKILL_LIMITS.files) throw new SkillBundleError(`파일이 너무 많습니다 (한도 ${SKILL_LIMITS.files}개).`);
        files.push({ path, bytes: readFileSync(target), executable: (stat.mode & 0o111) !== 0 });
      } else {
        throw new SkillBundleError(`일반 파일이 아닙니다: ${path}`);
      }
    }
  };
  const root = lstatSync(directory);
  if (root.isSymbolicLink() || !root.isDirectory()) throw new SkillBundleError("스킬 폴더가 폴더가 아니거나 링크입니다.");
  walk(directory, "");
  assertBundle(files);
  return files;
}
