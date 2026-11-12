import { createHash } from "node:crypto";
import { lstat, realpath } from "node:fs/promises";
import { isAbsolute, relative } from "node:path";

import type { GitActionResult } from "../shared/bridge";
import { isConflictStatus } from "../shared/gitChange";
import type { GitHunkAction, GitHunkDiff } from "../shared/gitHunks";
import { anyContentFilter, failureMessage, parsePorcelainStatus, repositoryFile, runGit } from "./gitStatus";

const DIFF_LIMIT = 1 << 20;
const STALE_MESSAGE = "변경 내용이 달라졌습니다. 파일 변경을 새로 고치십시오.";

/** Only plans created in main can be executed. The renderer never supplies a patch. */
export interface GitHunkPlan {
  readonly path: string;
  readonly action: GitHunkAction;
}

interface HunkSnapshot {
  root: string;
  path: string;
  fingerprint: string;
  header: string;
  hunks: GitHunkDiff["hunks"];
}

const plans = new WeakMap<GitHunkPlan, { snapshot: HunkSnapshot; hunkId: string }>();

function digest(value: string): string {
  return createHash("sha256").update(value).digest("hex");
}

async function snapshot(cwdInput: unknown, pathInput: unknown): Promise<HunkSnapshot> {
  if (typeof pathInput !== "string" || /[\0\r\n]/.test(pathInput)) {
    throw new Error("파일 경로가 올바르지 않습니다.");
  }
  const file = await repositoryFile(cwdInput, pathInput);
  const status = await runGit(file.root, ["status", "--porcelain=v1", "-z", "--untracked-files=normal"], 4 << 20);
  if (status.code !== 0 || status.truncated) throw new Error(failureMessage(status, "파일 상태를 모두 읽을 수 없습니다."));
  const change = parsePorcelainStatus(status.stdout).files.find((entry) => entry.path === file.relativePath);
  if (!change) throw new Error("스테이지하지 않은 변경이 없습니다.");
  if (isConflictStatus(change.status)) throw new Error("충돌 상태의 파일입니다. 터미널에서 해결하십시오.");
  if (change.untracked || change.directory) throw new Error("추적하지 않는 파일과 폴더는 파일 전체 동작을 사용하십시오.");
  if (/[RC]/.test(change.status)) throw new Error("이름을 바꾸거나 복사한 파일은 파일 전체 동작을 사용하십시오.");
  if (await anyContentFilter(file.root, [file.relativePath])) throw new Error("필터(LFS 등)가 있는 파일입니다. 터미널에서 git 으로 처리하십시오.");

  const index = await runGit(file.root, ["ls-files", "--stage", "-z", "--", file.pathspec], 4096);
  if (index.code !== 0 || index.truncated || !/^100(644|755) [a-f0-9]+ 0\t[^\0]+\0$/.test(index.stdout)) {
    throw new Error("일반 추적 파일만 부분 변경을 처리할 수 있습니다.");
  }
  const info = await lstat(file.target).catch((error: NodeJS.ErrnoException) => {
    if (error.code === "ENOENT") return null;
    throw error;
  });
  if (info && !info.isFile()) throw new Error("심볼릭 링크와 일반 파일이 아닌 경로는 부분 변경을 처리할 수 없습니다.");
  // A regular file may still be reached through a symlinked parent directory.
  if (info) {
    const canonicalRoot = await realpath(file.root);
    const fromRoot = relative(canonicalRoot, await realpath(file.target));
    if (fromRoot.startsWith("..") || isAbsolute(fromRoot)) throw new Error("작업 폴더 밖의 파일입니다.");
  }
  const diff = await runGit(file.root, [
    "-c", "diff.algorithm=myers", "-c", "diff.indentHeuristic=false",
    "diff", "--no-color", "--no-ext-diff", "--no-textconv", "--no-renames", "--no-relative",
    "--src-prefix=a/", "--dst-prefix=b/", "--unified=3", "--inter-hunk-context=0", "--", file.pathspec,
  ], DIFF_LIMIT);
  if (diff.code !== 0 || diff.truncated || diff.timedOut) throw new Error(failureMessage(diff, "변경 내용이 너무 크거나 읽을 수 없습니다. 파일 전체 동작을 사용하십시오."));
  if (/^(Binary files |GIT binary patch|old mode |new mode )/m.test(diff.stdout)) {
    throw new Error("바이너리 또는 파일 모드 변경은 파일 전체 동작을 사용하십시오.");
  }
  const starts = [...diff.stdout.matchAll(/^@@ -\d+(?:,\d+)? \+\d+(?:,\d+)? @@.*\n/gm)].map((match) => match.index);
  if (starts.length === 0) throw new Error("스테이지하지 않은 텍스트 변경이 없습니다.");
  const fingerprint = digest(JSON.stringify([file.relativePath, index.stdout, diff.stdout]));
  return {
    root: file.root,
    path: file.relativePath,
    fingerprint,
    header: diff.stdout.slice(0, starts[0]),
    hunks: starts.map((start, position) => ({
      id: digest(`${fingerprint}:${position}`),
      text: diff.stdout.slice(start, starts[position + 1] ?? diff.stdout.length),
    })),
  };
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export async function readGitHunks(cwdInput: unknown, pathInput: unknown): Promise<GitHunkDiff> {
  try {
    return { hunks: (await snapshot(cwdInput, pathInput)).hunks, message: null };
  } catch (error) {
    return { hunks: [], message: message(error) };
  }
}

export async function planGitHunkAction(
  cwdInput: unknown, action: unknown, pathInput: unknown, hunkId: unknown,
): Promise<{ ok: true; plan: GitHunkPlan } | { ok: false; message: string }> {
  try {
    if (action !== "stage" && action !== "discard") throw new Error("git 부분 변경 동작이 올바르지 않습니다.");
    if (typeof hunkId !== "string" || !/^[a-f0-9]{64}$/.test(hunkId)) throw new Error("부분 변경 ID가 올바르지 않습니다.");
    const current = await snapshot(cwdInput, pathInput);
    if (!current.hunks.some((hunk) => hunk.id === hunkId)) throw new Error(STALE_MESSAGE);
    const plan: GitHunkPlan = Object.freeze({ path: current.path, action });
    plans.set(plan, { snapshot: current, hunkId });
    return { ok: true, plan };
  } catch (error) {
    return { ok: false, message: message(error) };
  }
}

export async function executeGitHunkPlan(plan: GitHunkPlan): Promise<GitActionResult> {
  const saved = plans.get(plan);
  if (!saved) return { ok: false, message: "부분 변경 계획이 올바르지 않습니다." };
  plans.delete(plan);
  try {
    const current = await snapshot(saved.snapshot.root, saved.snapshot.path);
    if (current.fingerprint !== saved.snapshot.fingerprint) return { ok: false, message: STALE_MESSAGE };
    const hunk = current.hunks.find((entry) => entry.id === saved.hunkId);
    if (!hunk) return { ok: false, message: STALE_MESSAGE };
    const result = await runGit(current.root, [
      "apply", "--whitespace=nowarn", ...(plan.action === "stage" ? ["--cached"] : ["--reverse"]), "-",
    ], 4096, { stdin: current.header + hunk.text });
    return result.code === 0 && !result.timedOut && !result.truncated
      ? { ok: true }
      : { ok: false, message: failureMessage(result, "git 부분 변경 적용이 실패했습니다.") };
  } catch (error) {
    return { ok: false, message: message(error) };
  }
}
