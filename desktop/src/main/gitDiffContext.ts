import type { GitDiffSnapshot } from "../shared/bridge";
import type { GitDiffContextRequest, GitDiffContextResult } from "../shared/gitDiffContext";
import { contentLines, type CapturedContent } from "./gitSnapshotContent";
import { repositoryFile, snapshotGitFilePlan, type RepositoryFile } from "./gitStatus";

interface CachedSnapshot {
  readonly snapshot: GitDiffSnapshot;
  readonly file: RepositoryFile;
  readonly original: RepositoryFile | null;
  readonly guard: Awaited<ReturnType<typeof snapshotGitFilePlan>>;
  readonly old: CapturedContent;
  readonly new: CapturedContent;
}

/** Process-local read cache only. Durable selections belong to the attachment contract. */
const snapshots = new Map<string, CachedSnapshot>();
const CACHE_BYTES = 16 << 20;
const CACHE_ENTRIES = 16;

export function retainDiffSnapshot(entry: CachedSnapshot): void {
  snapshots.delete(entry.snapshot.snapshotId);
  snapshots.set(entry.snapshot.snapshotId, entry);
  let bytes = 0;
  for (const value of snapshots.values()) bytes += Buffer.byteLength(value.old.text) + Buffer.byteLength(value.new.text);
  while (snapshots.size > CACHE_ENTRIES || bytes > CACHE_BYTES) {
    const oldest = snapshots.entries().next().value;
    if (!oldest) break;
    bytes -= Buffer.byteLength(oldest[1].old.text) + Buffer.byteLength(oldest[1].new.text);
    snapshots.delete(oldest[0]);
  }
}

function parseRequest(input: unknown): GitDiffContextRequest {
  if (typeof input !== "object" || input === null ||
    !("snapshotId" in input) || typeof input.snapshotId !== "string" ||
    !("source" in input) || (input.source !== "head_worktree" && input.source !== "head_index" && input.source !== "index_worktree" && input.source !== "untracked" && input.source !== "worktree_file") ||
    !("side" in input) || (input.side !== "old" && input.side !== "new") ||
    !("startLine" in input) || typeof input.startLine !== "number" || !Number.isSafeInteger(input.startLine) ||
    !("endLine" in input) || typeof input.endLine !== "number" || !Number.isSafeInteger(input.endLine)) {
    throw new Error("Invalid snapshot context request.");
  }
  return { snapshotId: input.snapshotId, source: input.source, side: input.side, startLine: input.startLine, endLine: input.endLine };
}

export async function readGitDiffContext(cwd: unknown, input: unknown): Promise<GitDiffContextResult> {
  const request = parseRequest(input);
  const cached = snapshots.get(request.snapshotId);
  if (!cached) return { status: "unavailable", reason: "snapshot_expired" };
  const file = await repositoryFile(cwd, cached.file.relativePath);
  if (file.root !== cached.file.root || request.source !== cached.snapshot.source) {
    return { status: "unavailable", reason: "scope_mismatch" };
  }
  const content = cached[request.side];
  if (content.info.status === "unavailable") return content.info;
  if (request.startLine < 1 || request.endLine < request.startLine ||
    request.endLine - request.startLine >= 200 || request.endLine > content.info.lineCount) {
    return { status: "unavailable", reason: "range_limit" };
  }
  const lines = contentLines(content.text);
  const selected = lines.slice(request.startLine - 1, request.endLine);
  const text = selected.join("\n") + (request.endLine < lines.length || content.text.endsWith("\n") ? "\n" : "");
  if (Buffer.byteLength(text) > 64 << 10) return { status: "unavailable", reason: "range_limit" };
  const current = JSON.stringify(await snapshotGitFilePlan(file, cached.original)) === JSON.stringify(cached.guard);
  return { status: "available", ...request, text, current };
}
