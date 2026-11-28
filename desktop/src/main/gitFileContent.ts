import { constants } from "node:fs";
import { lstat, open, realpath } from "node:fs/promises";
import { isAbsolute, relative, sep } from "node:path";
import type { GitFileDiff } from "../shared/bridge";
export const FILE_DIFF_LIMIT = 1 << 20;

function canonicalFileIsInsideRoot(root: string, target: string): boolean {
  const fromRoot = relative(root, target);
  return fromRoot !== "" && fromRoot !== ".." && !fromRoot.startsWith(".." + sep) && !isAbsolute(fromRoot);
}

export async function readUntrackedFile(root: string, path: string): Promise<GitFileDiff> {
  // A lexical root check is insufficient: a symlink or junction can reach
  // outside the repository, including through an intermediate directory.
  const [canonicalRoot, info, canonicalTarget] = await Promise.all([
    realpath(root),
    lstat(path),
    realpath(path),
  ]);
  if (info.isSymbolicLink() || !info.isFile()) {
    throw new Error("심볼릭 링크 및 일반 파일이 아닌 경로는 내용을 표시하지 않습니다.");
  }
  if (!canonicalFileIsInsideRoot(canonicalRoot, canonicalTarget)) {
    throw new Error("저장소 외부 경로의 파일은 읽을 수 없습니다.");
  }

  // Resolve the canonical path before opening and refuse final symlinks on
  // Unix. Check descriptor identity after opening to detect replacements.
  // Nonblocking open also prevents a regular-file-to-FIFO race from waiting for a writer.
  const flags = constants.O_RDONLY | (process.platform === "win32" ? 0 :
    (constants.O_NOFOLLOW ?? 0) | (constants.O_NONBLOCK ?? 0));
  const handle = await open(canonicalTarget, flags);
  try {
    const [opened, current, currentTarget] = await Promise.all([
      handle.stat(),
      lstat(path),
      realpath(path),
    ]);
    if (!opened.isFile() || current.isSymbolicLink() || !current.isFile() ||
      opened.dev !== info.dev || opened.ino !== info.ino ||
      current.dev !== info.dev || current.ino !== info.ino ||
      currentTarget !== canonicalTarget || !canonicalFileIsInsideRoot(canonicalRoot, currentTarget)) {
      throw new Error("파일 경로가 읽기 도중 변경되어 내용을 표시하지 않습니다.");
    }
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
