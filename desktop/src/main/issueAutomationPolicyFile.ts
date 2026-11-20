// 정책 파일은 main이 고정 위치를 계산한 뒤 편집기로 연다. 원문을 읽거나 파일을 만들지 않는다.
import { posix, win32 } from "node:path";
import type { IssueAutomationPolicyFileRequest } from "../shared/bridge";
import { editorUrl } from "./gitStatus";

const POLICY_FILE_NAME = "issue-automation.toml";
const MAX_PATH_CHARACTERS = 32_768;

export interface IssueAutomationPolicyPathMetadata {
  isDirectory(): boolean;
  isFile(): boolean;
  isSymbolicLink(): boolean;
}

/** 파일 시스템과 OS 열기는 주입한다. 시험은 사용자의 파일이나 편집기에 접근하지 않는다. */
export interface IssueAutomationPolicyFileAccess {
  platform: NodeJS.Platform;
  environment: Readonly<Record<string, string | undefined>>;
  lstat(path: string): Promise<IssueAutomationPolicyPathMetadata>;
  getApplicationNameForProtocol(protocol: string): string;
  openExternal(url: string): Promise<void>;
  showItemInFolder(path: string): void;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasOnlyKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
  return Object.keys(value).every((key) => keys.includes(key));
}

function absolutePath(value: unknown, platform: NodeJS.Platform): string {
  if (typeof value !== "string" || value.length === 0 || value.length > MAX_PATH_CHARACTERS
    || /[\u0000-\u001f\u007f-\u009f]/.test(value)) {
    throw new Error("정책 파일 위치는 제어 문자가 없는 절대 경로여야 합니다.");
  }
  if (platform !== "win32") {
    if (!posix.isAbsolute(value)) throw new Error("정책 파일 위치는 절대 경로여야 합니다.");
    if (value.includes("\\") || value.split("/").some((part) => part === "." || part === "..")) {
      throw new Error("정책 파일 경로를 다른 위치로 정리할 수 없습니다. 점 경로나 역슬래시가 없는 실제 위치로 설정해 주세요.");
    }
    return posix.normalize(value);
  }
  let path = value.replaceAll("/", "\\");
  // Rust의 canonicalize가 반환하는 표준 확장 드라이브/UNC 표기는 같은 파일 경로로 비교한다.
  // 장치, named pipe, GLOBALROOT 같은 다른 namespace는 허용하지 않는다.
  if (/^\\\\\?\\UNC\\/i.test(path)) path = "\\\\" + path.slice(8);
  else if (/^\\\\\?\\[A-Za-z]:\\/.test(path)) path = path.slice(4);
  const unc = /^\\\\([^\\:]+)\\([^\\:]+)(?:\\|$)/.exec(path);
  const isUnc = unc !== null && ![".", "..", "?"].includes(unc[1]!) && ![".", ".."].includes(unc[2]!);
  if (/^\\\\[?.]\\/.test(path) || !(/^[A-Za-z]:\\/.test(path) || isUnc)) {
    throw new Error("정책 파일 위치는 드라이브 또는 공유가 있는 절대 경로여야 합니다.");
  }
  if (path.split("\\").some((part) => part === "." || part === ".." || /[. ]$/.test(part))) {
    throw new Error("정책 파일 경로를 다른 위치로 정리할 수 없습니다. 점 경로나 끝의 점·공백이 없는 실제 위치로 설정해 주세요.");
  }
  return win32.normalize(path);
}

function requestFromUnknown(value: unknown, platform: NodeJS.Platform): IssueAutomationPolicyFileRequest {
  if (!isRecord(value) || !hasOnlyKeys(value, ["workspace", "scope", "expectedPath", "host"]) || !isRecord(value.host)) {
    throw new Error("정책 파일 열기 요청이 올바르지 않습니다.");
  }
  if (value.host.kind === "remote") throw new Error("원격 호스트의 정책 파일은 이 컴퓨터에서 열 수 없습니다.");
  if (value.host.kind !== "local" || !hasOnlyKeys(value.host, ["kind"]) || (value.scope !== "user" && value.scope !== "project")) {
    throw new Error("정책 파일 열기 요청이 올바르지 않습니다.");
  }
  return {
    workspace: absolutePath(value.workspace, platform),
    expectedPath: absolutePath(value.expectedPath, platform),
    scope: value.scope,
    host: { kind: "local" },
  };
}

function userPolicyDirectory(access: IssueAutomationPolicyFileAccess): string {
  const paths = access.platform === "win32" ? win32 : posix;
  const override = access.environment.AKRA_HOME;
  if (override !== undefined && override.length > 0) return absolutePath(override, access.platform);
  // Rust configuration::akra_home_dir와 같다. os.homedir()의 별도 fallback은 쓰지 않는다.
  const profile = access.environment.USERPROFILE;
  const base = access.platform === "win32" && profile !== undefined && profile.length > 0
    ? profile : access.environment.HOME;
  if (base === undefined || base.length === 0) throw new Error("사용자 정책의 홈 경로를 확인할 수 없습니다.");
  return paths.join(absolutePath(base, access.platform), ".akra");
}

function samePath(left: string, right: string, platform: NodeJS.Platform): boolean {
  return platform === "win32" ? left.toLowerCase() === right.toLowerCase() : left === right;
}

async function inspect(path: string, access: IssueAutomationPolicyFileAccess): Promise<IssueAutomationPolicyPathMetadata | null> {
  try {
    return await access.lstat(path);
  } catch (error) {
    if (isRecord(error) && error.code === "ENOENT") return null;
    throw new Error("정책 파일 위치를 확인할 수 없습니다.");
  }
}

function isRealDirectory(metadata: IssueAutomationPolicyPathMetadata | null): boolean {
  return metadata !== null && !metadata.isSymbolicLink() && metadata.isDirectory();
}

async function nearestExistingDirectory(path: string, access: IssueAutomationPolicyFileAccess): Promise<string> {
  const paths = access.platform === "win32" ? win32 : posix;
  let candidate = path;
  for (;;) {
    const metadata = await inspect(candidate, access);
    if (metadata !== null) {
      if (!isRealDirectory(metadata)) throw new Error("정책 파일의 부모는 링크가 아닌 실제 폴더여야 합니다.");
      return candidate;
    }
    const parent = paths.dirname(candidate);
    if (parent === candidate) throw new Error("정책 파일의 부모 폴더를 확인할 수 없습니다.");
    candidate = parent;
  }
}

export async function openIssueAutomationPolicyFile(input: unknown, access: IssueAutomationPolicyFileAccess): Promise<void> {
  const request = requestFromUnknown(input, access.platform);
  const paths = access.platform === "win32" ? win32 : posix;
  const directory = request.scope === "user" ? userPolicyDirectory(access) : paths.join(request.workspace, ".akra");
  const target = paths.join(directory, POLICY_FILE_NAME);
  if (!samePath(request.expectedPath, target, access.platform)) {
    throw new Error("표시된 정책 파일 위치가 현재 설정과 다릅니다. 정책을 다시 읽어 주세요.");
  }
  if (!isRealDirectory(await inspect(request.workspace, access))) {
    throw new Error("작업 폴더는 링크가 아닌 실제 폴더여야 합니다.");
  }
  // .akra/AKRA_HOME의 링크를 leaf보다 먼저 거절한다. 누락된 부모도 실제 존재하는 폴더까지만 거슬러 확인한다.
  const existingParent = await nearestExistingDirectory(directory, access);
  const leaf = await inspect(target, access);
  if (leaf !== null && (leaf.isSymbolicLink() || !leaf.isFile())) {
    throw new Error("정책 파일은 링크가 아닌 일반 파일이어야 합니다.");
  }
  const reveal = leaf === null ? existingParent : target;
  try {
    if (access.getApplicationNameForProtocol("vscode://")) {
      await access.openExternal(editorUrl(target));
      return;
    }
  } catch {
    // 편집기 등록이 사라졌거나 실행 요청이 실패하면 검증한 파일/부모 위치를 보여 준다.
  }
  try {
    access.showItemInFolder(reveal);
  } catch {
    throw new Error("정책 파일 위치를 열지 못했습니다.");
  }
}
