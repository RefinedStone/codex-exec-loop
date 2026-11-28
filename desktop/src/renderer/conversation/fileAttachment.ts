import type { GitDiffSource, GitFileDiff } from "../../shared/bridge";

export const ATTACHMENT_LIMIT = { count: 8, lines: 100, bytes: 16_384, totalLines: 300, totalBytes: 49_152 } as const;
export type DraftTarget = { readonly hostId: string; readonly worktree: string; readonly threadId: string };
type Selection = { readonly startLine: number; readonly endLine: number };
export type FileAttachment = Readonly<Selection & {
  schemaVersion: 1; attachmentId: string; createdAt: string; draftRevision: number;
  origin: DraftTarget; turnId: string | null; repositoryId: string; worktreeId: string;
  sourceKind: GitDiffSource; snapshotId: string; filePath: string; oldPath: string | null;
  headOid: string | null; manifestId: string; side: "old" | "new"; fingerprint: string;
  selected: string; before: string; after: string; diffExcerpt: string; feedback: string;
  partial: boolean; truncated: boolean; included: boolean; staleReason: string | null; useOriginal: boolean;
}>;

export function targetKey(target: DraftTarget): string {
  return JSON.stringify(["file-draft-v1", target.hostId, target.worktree, target.threadId]);
}
const bytes = (value: string) => new TextEncoder().encode(value).length;
export class AttachmentError extends Error {}
const reduceSelection = "첨부 한도를 초과했습니다. 범위를 줄이거나 첨부를 제외하세요.";

export async function captureSelection(diff: GitFileDiff, range: Selection, origin: {
  target: DraftTarget; turnId: string | null; manifestId: string; draftRevision: number;
}): Promise<FileAttachment> {
  const snapshot = diff.snapshot;
  if (!snapshot?.worktreeId || !snapshot.repositoryId || snapshot.source !== "worktree_file" ||
      snapshot.contentStatus !== "complete" || diff.kind !== "content" || diff.truncated || diff.text.includes("\uFFFD")) {
    throw new AttachmentError("완전한 UTF-8 파일(1 MiB 이하)에서 범위를 선택하세요.");
  }
  // Keep original CRLF and the presence/absence of the final newline.
  const lines = diff.text.match(/[^\n]*\n|[^\n]+$/g) ?? [];
  const { startLine, endLine } = range;
  if (!Number.isSafeInteger(startLine) || !Number.isSafeInteger(endLine) || startLine < 1 || endLine < startLine || endLine > lines.length) {
    throw new AttachmentError(`파일의 실제 줄 범위(1–${lines.length})를 선택하세요.`);
  }
  if (endLine - startLine + 1 > ATTACHMENT_LIMIT.lines) throw new AttachmentError(reduceSelection);
  const selected = lines.slice(startLine - 1, endLine).join("");
  const before = lines[startLine - 2] ?? "", after = lines[endLine] ?? "";
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(selected));
  const attachment: FileAttachment = {
    schemaVersion: 1, attachmentId: crypto.randomUUID(), createdAt: new Date().toISOString(), draftRevision: origin.draftRevision,
    origin: origin.target, turnId: origin.turnId, repositoryId: snapshot.repositoryId, worktreeId: snapshot.worktreeId,
    sourceKind: snapshot.source, snapshotId: snapshot.snapshotId, filePath: snapshot.path, oldPath: snapshot.originalPath,
    headOid: snapshot.headOid, manifestId: origin.manifestId, side: "new", ...range,
    fingerprint: Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join(""),
    selected, before, after, diffExcerpt: "", feedback: "", partial: startLine > 1 || endLine < lines.length,
    truncated: false, included: true, staleReason: null, useOriginal: false,
  };
  checkBundle([attachment]);
  return attachment;
}

export function checkBundle(items: readonly FileAttachment[]): void {
  let totalBytes = 0, totalLines = 0;
  for (const item of items.filter((entry) => entry.included)) {
    const content = item.before + item.selected + item.after + item.diffExcerpt + item.feedback;
    const size = bytes(content);
    const lines = (content.match(/\n/g)?.length ?? 0) + (content.endsWith("\n") ? 0 : 1);
    if (size > ATTACHMENT_LIMIT.bytes || item.endLine - item.startLine + 1 > ATTACHMENT_LIMIT.lines) throw new AttachmentError(reduceSelection);
    totalBytes += size; totalLines += lines;
  }
  if (items.length > ATTACHMENT_LIMIT.count || totalBytes > ATTACHMENT_LIMIT.totalBytes || totalLines > ATTACHMENT_LIMIT.totalLines) {
    throw new AttachmentError(reduceSelection);
  }
}

/** JSON values, never interpolated fence delimiters. The preview is the wire text. */
export function fileMessage(instruction: string, items: readonly FileAttachment[]): string {
  const quotedContext = items.filter(item => item.included).map(item => ({
    filePath: item.filePath, sourceKind: item.sourceKind, side: item.side,
    startLine: item.startLine, endLine: item.endLine, before: item.before, selected: item.selected, after: item.after,
    diffExcerpt: item.diffExcerpt, feedback: item.feedback, partial: item.partial, useOriginal: item.useOriginal,
    ...(/^[a-f0-9]{64}$/.test(item.manifestId) ? { manifestId: item.manifestId } : {}),
  }));
  return quotedContext.length ? JSON.stringify({ instruction, quotedContext }, null, 2) : instruction;
}

const object = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null;
const optionalText = (value: unknown) => value === null || typeof value === "string";
export function isFileAttachment(item: unknown): item is FileAttachment {
  return object(item) && item.schemaVersion === 1 && Number.isSafeInteger(item.draftRevision)
    && ["attachmentId", "createdAt", "repositoryId", "worktreeId", "snapshotId", "filePath", "manifestId", "fingerprint",
      "selected", "before", "after", "diffExcerpt", "feedback"].every((key) => typeof item[key] === "string")
    && object(item.origin) && typeof item.origin.hostId === "string" && typeof item.origin.threadId === "string" && typeof item.origin.worktree === "string"
    && (item.side === "new" || item.side === "old")
    && ["head_worktree", "head_index", "index_worktree", "untracked", "worktree_file"].some((kind) => kind === item.sourceKind)
    && typeof item.startLine === "number" && Number.isSafeInteger(item.startLine) && item.startLine > 0
    && typeof item.endLine === "number" && Number.isSafeInteger(item.endLine) && item.endLine >= item.startLine
    && [item.turnId, item.oldPath, item.headOid, item.staleReason].every(optionalText)
    && ["partial", "truncated", "included", "useOriginal"].every((key) => typeof item[key] === "boolean");
}
