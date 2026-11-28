import { createHash } from "node:crypto";
import { lstat } from "node:fs/promises";
import type { GitSnapshotContent } from "../shared/gitDiffContext";
import { FILE_DIFF_LIMIT, readUntrackedFile } from "./gitFileContent";
import { failureMessage, runGit, type RepositoryFile } from "./gitStatus";

export interface CapturedContent {
  readonly info: GitSnapshotContent;
  readonly text: string;
}

export function contentLines(text: string): string[] {
  if (text === "") return [];
  const lines = text.split("\n");
  if (lines.at(-1) === "") lines.pop();
  return lines;
}

export function capturedText(text: string): CapturedContent {
  const crlf = text.match(/\r\n/g)?.length ?? 0;
  const lf = text.match(/\n/g)?.length ?? 0;
  return { text, info: { status: "complete", hash: createHash("sha256").update(text).digest("hex"),
    lineCount: contentLines(text).length, lineEnding: lf === 0 ? "none" : crlf === 0 ? "lf" : crlf === lf ? "crlf" : "mixed" } };
}

export async function captureBlob(file: RepositoryFile, revision: string | null): Promise<CapturedContent> {
  if (revision === null) return capturedText("");
  const result = revision === "index"
    ? await runGit(file.root, ["ls-files", "--stage", "-z", "--", file.pathspec], 4096)
    : await runGit(file.root, ["ls-tree", "-z", revision, "--", file.pathspec], 4096);
  if (result.code !== 0 || result.truncated || result.timedOut) throw new Error(failureMessage(result, "Cannot read blob identity."));
  if (result.stdout === "") return capturedText("");
  const match = /^(100644|100755) (?:blob )?([a-f0-9]+)(?: 0)?\t/.exec(result.stdout);
  const oid = match?.[2];
  if (!oid) return { text: "", info: { status: "unavailable", reason: "unsupported_entry" } };
  const blob = await runGit(file.root, ["cat-file", "blob", oid], FILE_DIFF_LIMIT);
  if (blob.code !== 0 || blob.timedOut) throw new Error(failureMessage(blob, "Cannot read blob."));
  if (blob.truncated) return { text: "", info: { status: "unavailable", reason: "size_limit" } };
  if (blob.stdout.includes("\0")) return { text: "", info: { status: "unavailable", reason: "binary" } };
  // The Git adapter decodes UTF-8. Reject replacement characters rather than inventing source bytes.
  if (blob.stdout.includes("\uFFFD")) return { text: "", info: { status: "unavailable", reason: "invalid_utf8" } };
  return capturedText(blob.stdout);
}

export async function captureWorkingFile(file: RepositoryFile): Promise<CapturedContent> {
  const info = await lstat(file.target).catch((error: NodeJS.ErrnoException) => {
    if (error.code === "ENOENT") return null;
    throw error;
  });
  if (info === null) return capturedText("");
  if (!info.isFile() || info.isSymbolicLink()) return { text: "", info: { status: "unavailable", reason: "unsupported_entry" } };
  if (info.size > FILE_DIFF_LIMIT) return { text: "", info: { status: "unavailable", reason: "size_limit" } };
  const result = await readUntrackedFile(file.root, file.target);
  switch (result.kind) {
    case "binary": return { text: "", info: { status: "unavailable", reason: "binary" } };
    case "gitDiff": throw new Error("Expected file content.");
    case "content":
      if (result.truncated) return { text: "", info: { status: "unavailable", reason: "size_limit" } };
      if (result.text.includes("\uFFFD")) return { text: "", info: { status: "unavailable", reason: "invalid_utf8" } };
      return capturedText(result.text);
  }
}
