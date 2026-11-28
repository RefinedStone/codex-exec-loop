import { useEffect, useState } from "react";
import { create } from "zustand";
import type { GitDiffSnapshot } from "../../shared/bridge";
import type { GitDiffSide } from "../../shared/gitDiffContext";
import { setFileList, useUiStore } from "../session/uiStore";
import { openThreadCwd, useDesktopStore } from "../session/desktopStore";
import { currentFileTarget, readManifest, type ManifestRequest } from "./attachmentSource";
import { ATTACHMENT_LIMIT, AttachmentError, captureAttachment, targetKey, type DraftTarget, type FileAttachment } from "./fileAttachment";

type Range = { readonly side: GitDiffSide; readonly startLine: number; readonly endLine: number };
type Offer = { readonly snapshot: GitDiffSnapshot; readonly range: Range; readonly target: DraftTarget;
  readonly turnId: string | null; readonly diffExcerpt: string };
// Only the open editor is transient. Committed feedback uses the existing scoped file draft.
const useEditor = create<{ readonly offer: Offer | null }>(() => ({ offer: null }));

export function DiffFeedbackAction({ cwd, snapshot, selection, coordinates }: {
  readonly cwd: string; readonly snapshot: GitDiffSnapshot; readonly selection: Range;
  readonly coordinates: readonly { readonly hunk: boolean; readonly text: string; readonly oldLine: number | null; readonly newLine: number | null }[];
}) {
  const threadView = useUiStore(s => s.view === "thread");
  const writable = useDesktopStore(s => s.connection?.phase === "connected" && !s.writerElsewhere &&
    s.openThreadId !== null && s.openThreadId === s.transcript?.threadId && openThreadCwd(s) === cwd);
  const offer = () => {
    const target = currentFileTarget(), state = useDesktopStore.getState();
    if (!target || target.worktree !== cwd || state.writerElsewhere) return;
    const diffExcerpt = coordinates.flatMap(row => {
      const line = selection.side === "old" ? row.oldLine : row.newLine;
      if (row.hunk || line === null || line < selection.startLine || line > selection.endLine) return [];
      return [(row.oldLine === null ? "+" : row.newLine === null ? "-" : " ") + row.text];
    }).join("\n");
    useEditor.setState({ offer: { snapshot, range: selection, target, diffExcerpt, turnId: state.transcript?.turns.at(-1)?.id ?? null } });
  };
  const content = snapshot.contents?.[selection.side];
  const valid = content?.status === "complete" && selection.startLine >= 1 && selection.endLine <= content.lineCount &&
    selection.endLine >= selection.startLine && selection.endLine - selection.startLine < ATTACHMENT_LIMIT.lines &&
    snapshot.contentStatus !== "binary" && snapshot.contentStatus !== "truncated";
  return <button type="button" className="button" disabled={!valid || !threadView || !writable} onClick={offer}>AI에게 피드백</button>;
}

async function readQuote(offer: Offer, request: ManifestRequest): Promise<FileAttachment> {
  const { range, snapshot, target } = offer;
  const content = snapshot.contents?.[range.side];
  if (content?.status !== "complete") throw new AttachmentError("선택한 면의 원문을 확인할 수 없습니다.");
  const first = Math.max(1, range.startLine - 1), last = Math.min(content.lineCount, range.endLine + 1);
  const [result, manifest] = await Promise.all([
    window.akra.gitDiffContext(target.worktree, { source: snapshot.source, snapshotId: snapshot.snapshotId,
      side: range.side, startLine: first, endLine: last }),
    readManifest(request, target, { path: snapshot.path }),
  ]);
  if (result.status !== "available") throw new AttachmentError(`원래 범위를 읽을 수 없습니다: ${result.reason}`);
  const lines = result.text.match(/[^\n]*\n|[^\n]+$/g) ?? [];
  const selected = lines.slice(range.startLine - first, range.endLine - first + 1).join("");
  return captureAttachment(snapshot, { target, turnId: offer.turnId, manifestId: manifest.manifestLedger.id,
    manifestLedger: manifest.manifestLedger, draftRevision: Date.now() },
    { ...range, selected, before: first < range.startLine ? lines[0] ?? "" : "",
      after: last > range.endLine ? lines.at(-1) ?? "" : "", partial: range.startLine > 1 || range.endLine < content.lineCount,
      diffExcerpt: offer.diffExcerpt, staleReason: result.current ? null : "선택 후 원본 변경" });
}

export function DiffFeedbackEditor({ target, request, disabled }: {
  readonly target: DraftTarget; readonly request: ManifestRequest; readonly disabled: boolean;
}) {
  const offer = useEditor(s => s.offer);
  const key = targetKey(target);
  if (!offer || targetKey(offer.target) !== key) return null;
  return <FeedbackForm key={JSON.stringify([key, offer.snapshot.snapshotId, offer.range])} offer={offer} request={request} disabled={disabled} />;
}

function FeedbackForm({ offer, request, disabled }: {
  readonly offer: Offer; readonly request: ManifestRequest; readonly disabled: boolean;
}) {
  const [item, setItem] = useState<FileAttachment | null>(null);
  const [feedback, setFeedback] = useState("");
  const [error, setError] = useState("");
  const key = targetKey(offer.target);
  useEffect(() => {
    let active = true;
    setItem(null); setError(""); setFeedback("");
    void readQuote(offer, request).then(value => { if (active) setItem(value); },
      (cause: unknown) => { if (active) setError(cause instanceof Error ? cause.message : String(cause)); });
    return () => { active = false; };
  }, [offer, request]);
  const close = () => useEditor.setState({ offer: null });
  const add = () => {
    const live = currentFileTarget();
    if (!item || !live || targetKey(live) !== key || disabled || useEditor.getState().offer !== offer) return;
    try {
      setFileList(key, [...(useUiStore.getState().fileLists[key] ?? []), { ...item, feedback, draftRevision: Date.now() }]);
      close();
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
  };
  return <fieldset className="diff-feedback-editor" disabled={disabled}><legend>AI에게 피드백</legend>
    <p>{offer.snapshot.path} · {offer.range.side === "old" ? "이전" : "이후"} {offer.range.startLine}–{offer.range.endLine}</p>
    {!item && !error && <p role="status">원래 코드 읽는 중…</p>}
    {item && <><pre data-testid="feedback-selection">{item.selected}</pre>
      <label>수정 지시<textarea autoFocus value={feedback} onChange={e => setFeedback(e.target.value)} /></label></>}
    {error && <p role="alert">{error}</p>}
    <button type="button" disabled={!item || !feedback.trim()} onClick={add}>지시에 담기</button>
    <button type="button" onClick={close}>피드백 취소</button>
  </fieldset>;
}
