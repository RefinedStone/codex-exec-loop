import type { GitDiffSource, GitFileDiff } from "../../shared/bridge";
import { openThreadCwd, useDesktopStore } from "../session/desktopStore";
import { useUiStore } from "../session/uiStore";
import { AttachmentError, targetKey, type DraftTarget, type FileAttachment } from "./fileAttachment";

export type ManifestRequest = <T>(method: string, params: unknown) => Promise<T>;
export function currentFileTarget(): DraftTarget | null {
  const state = useDesktopStore.getState(), worktree = openThreadCwd(state);
  return useUiStore.getState().view === "thread" && state.connection?.phase === "connected"
    && state.openThreadId && state.openThreadId === state.transcript?.threadId && worktree
    ? { hostId: state.connection.endpoint, worktree, threadId: state.openThreadId } : null;
}

export function threadDraftKey(threadId: string): string {
  const target = currentFileTarget();
  return target?.threadId === threadId ? targetKey(target) : threadId;
}

type Source = { readonly path: string; readonly previousId?: string; readonly kind?: GitDiffSource };

export async function readSource(request: ManifestRequest, target: DraftTarget, source: Source): Promise<{
  diff: GitFileDiff; manifestId: string; changed: boolean;
}> {
  const [diff, manifest] = await Promise.all([
    window.akra.gitFileDiff(target.worktree, source.path, source.kind === "untracked", source.kind ?? "worktree_file"),
    readManifest(request, target, source),
  ]);
  return { diff, ...manifest };
}

export async function readManifest(request: ManifestRequest, target: DraftTarget, source: Source) {
  const result = await request<unknown>("akra/context/manifest", { cwd: target.worktree, paths: [source.path], previousId: source.previousId });
  if (!result || typeof result !== "object" || !("manifest" in result) || !result.manifest ||
      typeof result.manifest !== "object" || !("id" in result.manifest) || typeof result.manifest.id !== "string" ||
      !("changed" in result) || typeof result.changed !== "boolean") throw new AttachmentError("자료 manifest를 확인하지 못했습니다.");
  return { manifestId: result.manifest.id, changed: result.changed };
}

/** Observe the original scope only. Never resolve a remote or copied path against the current host. */
export async function reviewFiles(request: ManifestRequest, files: readonly FileAttachment[], target: DraftTarget): Promise<readonly FileAttachment[]> {
  return Promise.all(files.map(async (item) => {
    if (!item.included) return item;
    let reason: string | null = null;
    if (item.origin.hostId !== target.hostId) reason = "다른 호스트의 원래 선택";
    else {
      try {
        const source = await readSource(request, item.origin, { path: item.filePath, kind: item.sourceKind, previousId: item.manifestId });
        if (source.changed || source.diff.snapshot?.snapshotId !== item.snapshotId) {
          reason = `파일/index/ref 또는 자료 manifest 변경: ${source.diff.snapshot?.snapshotId ?? "?"} / ${source.manifestId}`;
        } else if (targetKey(item.origin) !== targetKey(target)) reason = "다른 대상에 복사한 선택";
      } catch (error) {
        reason = error instanceof Error ? error.message : String(error);
      }
    }
    return reason === item.staleReason ? item : { ...item, staleReason: reason, useOriginal: false };
  }));
}
