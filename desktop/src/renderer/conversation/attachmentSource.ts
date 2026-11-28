import type { GitDiffSource, GitFileDiff } from "../../shared/bridge";
import { openThreadCwd, useDesktopStore } from "../session/desktopStore";
import { useUiStore } from "../session/uiStore";
import { AttachmentError, targetKey, type DraftTarget, type FileAttachment } from "./fileAttachment";
import { isManifestLedger, type ManifestLedger } from "./attachmentManifest";

export type ManifestRequest = <T>(method: string, params: unknown) => Promise<T>;
export function currentFileTarget(requireConnection = true): DraftTarget | null {
  const state = useDesktopStore.getState(), worktree = openThreadCwd(state);
  return useUiStore.getState().view === "thread" && state.connection && (!requireConnection || state.connection.phase === "connected")
    && state.openThreadId && state.openThreadId === state.transcript?.threadId && worktree
    ? { hostId: state.connection.endpoint, worktree, threadId: state.openThreadId } : null;
}

export function threadDraftKey(threadId: string): string {
  const target = currentFileTarget(false);
  return target?.threadId === threadId ? targetKey(target) : threadId;
}

type Source = { readonly path: string; readonly previousId?: string; readonly kind?: GitDiffSource };

export async function readSource(request: ManifestRequest, target: DraftTarget, source: Source): Promise<{
  diff: GitFileDiff; manifestLedger: ManifestLedger; changed: boolean;
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
      typeof result.manifest !== "object" || !("unverifiedCount" in result) ||
      !("changed" in result) || typeof result.changed !== "boolean") throw new AttachmentError("자료 manifest를 확인하지 못했습니다.");
  const manifestLedger = { ...result.manifest, unverifiedCount: result.unverifiedCount };
  if (!isManifestLedger(manifestLedger)) throw new AttachmentError("자료 manifest를 확인하지 못했습니다.");
  return {
    manifestLedger: { id: manifestLedger.id, unverifiedCount: manifestLedger.unverifiedCount,
      entries: manifestLedger.entries.map(({ kind, key, selected, reason, observedVersion }) => ({ kind, key, selected, reason, observedVersion })) },
    changed: result.changed,
  };
}

/** Observe the original scope only. Never resolve a remote or copied path against the current host. */
export async function reviewFiles(request: ManifestRequest, files: readonly FileAttachment[], target: DraftTarget): Promise<readonly FileAttachment[]> {
  return Promise.all(files.map(async (item) => {
    if (!item.included) return item;
    let reason: string | null = null;
    let original = item.manifestLedger, current = item.currentLedger;
    if (item.origin.hostId !== target.hostId) reason = "다른 호스트의 원래 선택";
    else {
      try {
        const source = await readSource(request, item.origin, { path: item.filePath, kind: item.sourceKind, previousId: item.manifestId });
        const observed = source.manifestLedger;
        if (observed.id === item.manifestId) original ??= observed;
        current = observed.id === item.manifestId ? undefined : current?.id === observed.id ? current : observed;
        if (source.changed || current || source.diff.snapshot?.snapshotId !== item.snapshotId) {
          reason = `파일/index/ref 또는 자료 manifest 변경: ${source.diff.snapshot?.snapshotId ?? "?"} / ${observed.id}`;
        } else if (targetKey(item.origin) !== targetKey(target)) reason = "다른 대상에 복사한 선택";
      } catch (error) {
        throw new AttachmentError(`자료 버전을 비교하지 못했습니다. 초안을 보존했습니다: ${error instanceof Error ? error.message : String(error)}`);
      }
    }
    if (reason === item.staleReason && original === item.manifestLedger && current === item.currentLedger) return item;
    const { manifestLedger, currentLedger, ...quote } = item;
    return { ...quote, staleReason: reason, useOriginal: reason === item.staleReason && item.useOriginal,
      ...(original && { manifestLedger: original }), ...(current && { currentLedger: current }) };
  }));
}
