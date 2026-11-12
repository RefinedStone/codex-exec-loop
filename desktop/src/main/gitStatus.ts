// 스레드 cwd 의 작업 트리 상태를 main 에서 git 으로 읽는다 (R1). 파일 하나의 스테이지, 스테이지 취소, 되돌리기도 한다 (R10).
// 공식 `gitDiffToRemote` 는 쓰지 않는다. untracked 파일까지 한 번에 diff 로 만들어서, 큰 파일(실측 1.3~3.9 GB)이
// 있는 폴더에서는 응답이 접속 한도를 넘고 Gateway 세션 전체가 끊겼다 (2026-11-06). 여기서는 목록만 먼저 읽고,
// 파일 diff 는 펼칠 때 하나씩, 크기 한도 안에서 읽는다.
import { spawn } from "node:child_process";
import { open, stat } from "node:fs/promises";
import { isAbsolute, normalize, relative, resolve, sep } from "node:path";

import type { GitActionResult, GitChangedFile, GitFileAction, GitFileDiff, GitStatusResult } from "../shared/bridge";
import { isConflictStatus, stageState } from "../shared/gitChange";

const GIT_TIMEOUT_MS = 15_000;
/** 목록 출력의 한도이다. 이보다 크면 잘린 목록을 보여 준다. */
const STATUS_OUTPUT_LIMIT = 4 << 20;
/** 파일 하나의 diff 한도이다. 넘으면 앞부분만 보여 준다. */
const FILE_DIFF_LIMIT = 1 << 20;

export interface GitOutput {
  stdout: string;
  code: number | null;
  truncated: boolean;
  timedOut: boolean;
  stderr: string;
}

export function failureMessage(output: GitOutput, fallback: string): string {
  if (output.timedOut) {
    return `git 이 ${GIT_TIMEOUT_MS / 1000}초 안에 끝나지 않았습니다.`;
  }
  return output.stderr.trim() || fallback;
}

/**
 * git 이 다른 프로그램을 띄우지 않게 하는 설정이다. Windows 에서 콘솔 없이 실행한 git 이 `git-lfs` 같은 콘솔
 * 프로그램을 띄우면 그 프로그램마다 터미널 창이 새로 뜬다 (2026-11-06, 전역 `filter.lfs.process` 가 있는 PC).
 * shortcut: LFS 필터를 끄므로, 내용은 같고 시각만 바뀐 LFS 파일이 "수정" 으로 보일 수 있다.
 */
export const NO_HELPER_PROGRAMS = [
  "-c", "filter.lfs.process=",
  "-c", "filter.lfs.clean=",
  "-c", "filter.lfs.smudge=",
  "-c", "filter.lfs.required=false",
  "-c", "core.fsmonitor=false",
];

/** git 을 셸 없이 실행하고 출력은 한도까지만 모은다. Windows 에서 콘솔 창을 띄우지 않는다. */
export function runGit(cwd: string, args: string[], limit: number): Promise<GitOutput> {
  return new Promise((resolvePromise, reject) => {
    const child = spawn("git", ["-c", "core.quotePath=false", ...NO_HELPER_PROGRAMS, ...args], { cwd, windowsHide: true });
    const chunks: Buffer[] = [];
    let size = 0;
    let truncated = false;
    let stderr = "";
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill();
    }, GIT_TIMEOUT_MS);
    child.stdout.on("data", (chunk: Buffer) => {
      if (truncated) return;
      chunks.push(chunk);
      size += chunk.length;
      if (size > limit) {
        truncated = true;
        child.kill();
      }
    });
    child.stderr.on("data", (chunk: Buffer) => {
      stderr = (stderr + chunk.toString("utf8")).slice(-2000);
    });
    child.on("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      const stdout = Buffer.concat(chunks).subarray(0, limit).toString("utf8");
      resolvePromise({ stdout, code: truncated ? 0 : code, truncated, timedOut, stderr });
    });
  });
}

/** `git status --porcelain=v1 -z --branch` 출력을 읽는다. 이름 바꾸기는 새 경로와 이전 경로가 함께 온다. */
export function parsePorcelainStatus(output: string): { branch: string | null; files: GitChangedFile[] } {
  const entries = output.split("\0");
  let branch: string | null = null;
  const files: GitChangedFile[] = [];
  for (let index = 0; index < entries.length; index += 1) {
    const entry = entries[index]!;
    if (entry.startsWith("## ")) {
      branch = entry.slice(3);
      continue;
    }
    if (entry.length < 4) continue;
    const status = entry.slice(0, 2);
    const path = entry.slice(3);
    const renamed = status.includes("R") || status.includes("C");
    const originalPath = renamed ? (entries[index + 1] ?? null) : null;
    if (renamed) index += 1;
    files.push({
      path,
      originalPath,
      status,
      untracked: status === "??",
      directory: path.endsWith("/"),
      added: null,
      removed: null,
    });
  }
  return { branch, files };
}

/** `git diff HEAD --numstat -z` 출력을 경로별 줄 수로 바꾼다. 바이너리 파일은 "-" 이므로 null 이다. */
export function parseNumstat(output: string): Map<string, { added: number | null; removed: number | null }> {
  const counts = new Map<string, { added: number | null; removed: number | null }>();
  const entries = output.split("\0");
  for (let index = 0; index < entries.length; index += 1) {
    const match = /^(-|\d+)\t(-|\d+)\t(.*)$/s.exec(entries[index]!);
    if (!match) continue;
    const value = {
      added: match[1] === "-" ? null : Number(match[1]),
      removed: match[2] === "-" ? null : Number(match[2]),
    };
    if (match[3] === "") {
      // 이름 바꾸기: 다음 두 항목이 이전 경로와 새 경로이다.
      counts.set(entries[index + 2] ?? "", value);
      index += 2;
    } else {
      counts.set(match[3]!, value);
    }
  }
  return counts;
}

export async function requireDirectory(cwd: unknown): Promise<string> {
  if (typeof cwd !== "string" || !isAbsolute(cwd)) {
    throw new Error("작업 폴더는 절대 경로여야 합니다.");
  }
  const info = await stat(cwd).catch(() => null);
  if (!info?.isDirectory()) {
    throw new Error("작업 폴더가 없습니다.");
  }
  return cwd;
}

/** 작업 폴더 안의 상대 경로만 받는다. renderer 가 보낸 경로로 폴더 밖 파일을 읽지 않게 한다. */
export function resolveInside(root: string, path: unknown): string {
  if (typeof path !== "string" || path.length === 0 || isAbsolute(path)) {
    throw new Error("파일 경로가 올바르지 않습니다.");
  }
  const target = resolve(root, normalize(path));
  const fromRoot = relative(root, target);
  if (fromRoot === "" || fromRoot.startsWith("..") || isAbsolute(fromRoot) || fromRoot.split(sep).includes("..")) {
    throw new Error("작업 폴더 밖의 파일입니다.");
  }
  return target;
}

export async function readGitStatus(cwdInput: unknown): Promise<GitStatusResult> {
  const cwd = await requireDirectory(cwdInput);
  const status = await runGit(cwd, ["status", "--porcelain=v1", "-z", "--branch", "--untracked-files=normal"], STATUS_OUTPUT_LIMIT);
  if (status.code !== 0) {
    return { ok: false, message: failureMessage(status, "git 저장소가 아닙니다.") };
  }
  const parsed = parsePorcelainStatus(status.stdout);
  // 첫 커밋 전에는 HEAD 가 없어서 실패한다. 그때는 줄 수 없이 목록만 보여 준다.
  const numstat = await runGit(cwd, ["diff", "HEAD", "--numstat", "-z", "--no-ext-diff", "--no-textconv"], STATUS_OUTPUT_LIMIT);
  const counts = numstat.code === 0 ? parseNumstat(numstat.stdout) : new Map();
  const files = parsed.files.map((file) => ({ ...file, ...(counts.get(file.path) ?? {}) }));
  return { ok: true, branch: parsed.branch, files, truncated: status.truncated };
}

async function readUntrackedFile(path: string): Promise<GitFileDiff> {
  const handle = await open(path, "r");
  try {
    const buffer = Buffer.alloc(FILE_DIFF_LIMIT + 1);
    const { bytesRead } = await handle.read(buffer, 0, buffer.length, 0);
    const content = buffer.subarray(0, Math.min(bytesRead, FILE_DIFF_LIMIT));
    if (content.includes(0)) {
      return { kind: "binary" };
    }
    return { kind: "content", text: content.toString("utf8"), truncated: bytesRead > FILE_DIFF_LIMIT };
  } finally {
    await handle.close();
  }
}

interface RepositoryFile {
  cwd: string;
  /** 저장소 루트의 절대 경로이다. */
  root: string;
  /** 루트 기준의 경로이다 (`/` 구분). */
  relativePath: string;
  /** 파일의 절대 경로이다. */
  target: string;
  /** 저장소 루트 기준의 git 경로 지정이다. */
  pathspec: string;
}

/**
 * renderer 가 보낸 경로를 저장소 안의 파일로 확인한다.
 * status 의 경로는 cwd 가 아니라 저장소 루트 기준이다. cwd 가 하위 폴더여도 루트에서 찾는다.
 */
async function repositoryFile(cwdInput: unknown, path: unknown): Promise<RepositoryFile> {
  const cwd = await requireDirectory(cwdInput);
  const top = await runGit(cwd, ["rev-parse", "--show-toplevel"], 4096);
  if (top.code !== 0) {
    throw new Error(failureMessage(top, "git 저장소가 아닙니다."));
  }
  const root = resolve(top.stdout.trim());
  const target = resolveInside(root, path);
  const relativePath = relative(root, target).split(sep).join("/");
  // literal: 경로의 `*` 같은 문자를 패턴으로 읽지 않는다.
  return { cwd, root, relativePath, target, pathspec: `:(top,literal)${relativePath}` };
}

/** 편집기로 열 파일의 절대 경로이다. 저장소 밖의 경로는 거절한다. */
export async function resolveGitFile(cwdInput: unknown, path: unknown): Promise<string> {
  return (await repositoryFile(cwdInput, path)).target;
}

const GIT_FILE_ACTIONS: readonly GitFileAction[] = ["stage", "unstage", "discard"];
const FILTER_MESSAGE = "필터(LFS 등)가 있는 파일입니다. 터미널에서 git 으로 처리하십시오.";
/** 폴더를 스테이지할 때 필터를 확인하는 파일 수의 한도이다. */
const FOLDER_FILE_LIMIT = 200;
const ATTRIBUTE_BATCH = 50;

/** 확인을 마친 git 동작이다. `planGitFileAction` 만 만든다. */
export interface GitFilePlan {
  cwd: string;
  commands: string[][];
  /** 확인 창에 보여 줄 경로이다 (저장소 루트 기준). */
  path: string;
  /** 이름을 바꾼 파일을 버릴 때 되살리는 이전 경로이다. */
  originalPath: string | null;
}

export type GitFilePlanResult = { ok: true; plan: GitFilePlan } | { ok: false; message: string };

function refuse(message: string): GitFilePlanResult {
  return { ok: false, message };
}

/** `discard` 가 실행할 git 명령이다. 상태(porcelain 두 글자)에 따라 다르다. 충돌 상태와 폴더는 받지 않는다. */
export function discardCommands(status: string, pathspec: string, originalPathspec: string | null): string[][] {
  const restore = (spec: string) => ["restore", "--staged", "--worktree", "--source=HEAD", "--", spec];
  if (status === "??") return [["clean", "-f", "--", pathspec]];
  // 새로 더한 파일과 복사한 파일은 HEAD 에 없다. 지우면 된다.
  if (status.startsWith("A") || status.startsWith("C")) return [["rm", "-f", "-q", "--", pathspec]];
  // 이름을 바꾼 파일은 새 경로를 지우고 이전 경로를 되살린다.
  if (status.startsWith("R") && originalPathspec) return [["rm", "-f", "-q", "--", pathspec], restore(originalPathspec)];
  return [restore(pathspec)];
}

/**
 * 이 파일의 지금 상태를 git 에서 읽는다. renderer 가 보낸 상태는 쓰지 않는다. 오래된 목록이나 꾸민 값으로 다른
 * 명령을 고르지 못하게 한다. 이름 바꾸기는 경로를 좁혀 읽으면 보이지 않으므로 전체 목록에서 찾는다.
 */
async function currentChange(file: RepositoryFile): Promise<GitChangedFile | null> {
  const status = await runGit(file.root, ["status", "--porcelain=v1", "-z", "--untracked-files=normal"], STATUS_OUTPUT_LIMIT);
  if (status.code !== 0) {
    throw new Error(failureMessage(status, "git status 가 실패했습니다."));
  }
  return parsePorcelainStatus(status.stdout).files.find((change) => change.path.replace(/\/$/, "") === file.relativePath) ?? null;
}

/**
 * 경로 중 하나라도 `filter` 속성(LFS 등)이 있는지 본다. 이 모듈은 git 이 필터 프로그램을 띄우지 않게 한다
 * (`NO_HELPER_PROGRAMS`). 그 상태로 스테이지하면 포인터가 아니라 원본이 들어가고, 되돌리면 포인터 파일이 작업 폴더에 써진다.
 */
async function anyContentFilter(root: string, relativePaths: readonly string[]): Promise<boolean> {
  for (let start = 0; start < relativePaths.length; start += ATTRIBUTE_BATCH) {
    // check-attr 는 경로 지정이 아니라 경로를 받는다. 그래서 루트에서 루트 기준 경로로 묻는다. 출력은 `경로 NUL 속성 NUL 값 NUL` 이다.
    const output = await runGit(root, ["check-attr", "-z", "filter", "--", ...relativePaths.slice(start, start + ATTRIBUTE_BATCH)], STATUS_OUTPUT_LIMIT);
    if (output.code !== 0 || output.truncated) {
      throw new Error(failureMessage(output, "git check-attr 가 실패했습니다."));
    }
    const fields = output.stdout.split("\0");
    for (let index = 2; index < fields.length; index += 3) {
      if (fields[index] !== "unspecified" && fields[index] !== "unset") return true;
    }
  }
  return false;
}

/** 새 폴더 안의 추적하지 않는 파일이다. 한도보다 많으면 null 이다. */
async function untrackedFilesIn(file: RepositoryFile): Promise<string[] | null> {
  const output = await runGit(file.root, ["ls-files", "-o", "--exclude-standard", "-z", "--", file.pathspec], STATUS_OUTPUT_LIMIT);
  if (output.code !== 0) {
    throw new Error(failureMessage(output, "git ls-files 가 실패했습니다."));
  }
  const paths = output.stdout.split("\0").filter((path) => path.length > 0);
  return output.truncated || paths.length > FOLDER_FILE_LIMIT ? null : paths;
}

/**
 * 파일 하나의 스테이지, 스테이지 취소, 변경 버리기를 확인하고 실행할 명령을 정한다. 아무것도 바꾸지 않는다.
 * shortcut: `filter` 속성이 있는 파일(LFS), 충돌 상태의 파일, 폴더 버리기는 거절한다. 필요하면 필터를 켠 git 실행과
 * 충돌 해결 화면을 따로 만든다.
 */
export async function planGitFileAction(cwdInput: unknown, action: unknown, path: unknown): Promise<GitFilePlanResult> {
  if (!GIT_FILE_ACTIONS.includes(action as GitFileAction)) {
    throw new Error("git 동작 요청이 올바르지 않습니다.");
  }
  const file = await repositoryFile(cwdInput, path);
  const change = await currentChange(file);
  if (!change) return refuse("바뀐 파일 목록에 없습니다. 목록을 새로 고치십시오.");
  if (isConflictStatus(change.status)) return refuse("충돌 상태의 파일입니다. 터미널에서 해결하십시오.");
  const original = change.originalPath ? await repositoryFile(file.root, change.originalPath) : null;
  const plan = (commands: string[][]): GitFilePlanResult => ({
    ok: true,
    plan: { cwd: file.cwd, commands, path: change.path, originalPath: change.originalPath },
  });

  if (action === "unstage") {
    if (!stageState(change).staged) return refuse("스테이지한 변경이 없습니다.");
    const specs = [...(original ? [original.pathspec] : []), file.pathspec];
    // 첫 커밋 전에는 HEAD 가 없어서 `restore --staged` 가 실패한다.
    const head = await runGit(file.root, ["rev-parse", "--verify", "-q", "HEAD"], 4096);
    return plan([head.code === 0 ? ["restore", "--staged", "--", ...specs] : ["rm", "--cached", "-q", "--", ...specs]]);
  }
  if (change.directory) {
    if (action === "discard") return refuse("폴더는 한 번에 버리지 않습니다. 안의 파일을 터미널에서 지우십시오.");
    const inside = await untrackedFilesIn(file);
    if (!inside) return refuse(`폴더 안의 파일이 ${FOLDER_FILE_LIMIT}개보다 많습니다. 터미널에서 스테이지하십시오.`);
    if (await anyContentFilter(file.root, inside)) return refuse(FILTER_MESSAGE);
    return plan([["add", "-A", "--", file.pathspec]]);
  }
  if (await anyContentFilter(file.root, [file.relativePath, ...(original ? [original.relativePath] : [])])) return refuse(FILTER_MESSAGE);
  if (action === "stage") return plan([["add", "-A", "--", file.pathspec]]);
  // 이전 경로에 새 파일이 생겼으면 되살릴 때 그 파일을 덮어쓴다.
  if (change.status.startsWith("R") && original && (await stat(original.target).catch(() => null))) {
    return refuse(`이전 경로(${change.originalPath})에 다른 파일이 있습니다. 그 파일을 옮긴 뒤에 다시 하십시오.`);
  }
  return plan(discardCommands(change.status, file.pathspec, original?.pathspec ?? null));
}

/** 확인을 마친 동작을 실행한다. `discard` 는 되돌릴 수 없다. 부르는 쪽이 사용자에게 먼저 확인한다. */
export async function executeGitFilePlan(plan: GitFilePlan): Promise<GitActionResult> {
  for (const args of plan.commands) {
    const output = await runGit(plan.cwd, args, 4096);
    if (output.code !== 0) {
      return { ok: false, message: failureMessage(output, `git ${args[0]} 가 실패했습니다.`) };
    }
  }
  return { ok: true };
}

/** 확인하고 바로 실행한다. 사용자 확인이 필요 없는 동작과 시험이 쓴다. */
export async function runGitFileAction(cwdInput: unknown, action: unknown, path: unknown): Promise<GitActionResult> {
  const planned = await planGitFileAction(cwdInput, action, path);
  return planned.ok ? executeGitFilePlan(planned.plan) : planned;
}

/** 파일을 VS Code 로 여는 주소이다. 드라이브 문자(`C:`)는 그대로 두고 나머지 구간만 인코딩한다. */
export function editorUrl(target: string): string {
  const parts = target.split(/[\\/]/);
  return `vscode://file/${parts.map((part, index) => (index === 0 && /^[A-Za-z]:$/.test(part) ? part : encodeURIComponent(part))).join("/")}`;
}

/** 파일 하나의 변경이다. 추적 중인 파일은 HEAD 대비 git diff, 새 파일은 내용이다. */
export async function readGitFileDiff(cwdInput: unknown, path: unknown, untracked: unknown): Promise<GitFileDiff> {
  const { cwd, target, pathspec } = await repositoryFile(cwdInput, path);
  if (untracked === true) {
    return readUntrackedFile(target);
  }
  const output = await runGit(cwd, ["diff", "HEAD", "--no-color", "--no-ext-diff", "--no-textconv", "--", pathspec], FILE_DIFF_LIMIT);
  if (output.code !== 0) {
    throw new Error(failureMessage(output, "git diff 가 실패했습니다."));
  }
  return { kind: "gitDiff", text: output.stdout, truncated: output.truncated };
}
