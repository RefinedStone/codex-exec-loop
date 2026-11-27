import { useState } from "react";
import "./fileAttachments.css";
import { useUiStore, setFileList } from "../session/uiStore";
import { readSource, type ManifestRequest } from "./attachmentSource";
import { captureSelection, fileMessage, targetKey, type DraftTarget, type FileAttachment } from "./fileAttachment";

export function FileAttachments({ target, turnId, request, disabled }: {
  target: DraftTarget; turnId: string | null; request: ManifestRequest; disabled: boolean;
}) {
  const [path, setPath] = useState(""), [start, setStart] = useState("1"), [end, setEnd] = useState("1");
  const [error, setError] = useState(""), [busy, setBusy] = useState(false);
  const key = targetKey(target);
  const all = useUiStore((s) => s.fileLists), files = all[key] ?? [];
  const draft = useUiStore((s) => s.drafts[key] ?? "");
  const add = (item: FileAttachment) => setFileList(key, [...(useUiStore.getState().fileLists[key] ?? []), item]);
  const change = (next: readonly FileAttachment[]) => {
    try { setFileList(key, next); setError(""); } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
  };
  const attach = async () => {
    setBusy(true); setError("");
    try {
      const { diff, manifestId } = await readSource(request, target, { path });
      add(await captureSelection(diff, { startLine: Number(start), endLine: Number(end) },
        { target, turnId, manifestId, draftRevision: Date.now() }));
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  };
  const copy = (item: FileAttachment) => {
    try { add({ ...item, attachmentId: crypto.randomUUID(), draftRevision: Date.now(), included: true,
      staleReason: "다른 대상에 복사한 선택", useOriginal: false }); setError(""); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
  };
  return <section aria-label="파일 첨부" className="file-attachments">
    <details><summary>파일·행 첨부 {files.length ? `(${files.length})` : ""}</summary>
      <fieldset disabled={disabled || busy}><legend>작업 파일 범위</legend>
        <label>저장소 상대 경로<input value={path} onChange={e => setPath(e.target.value)} /></label>
        <label>시작 줄<input type="number" min="1" value={start} onChange={e => setStart(e.target.value)} /></label>
        <label>끝 줄<input type="number" min="1" value={end} onChange={e => setEnd(e.target.value)} /></label>
        <button type="button" disabled={!path} onClick={() => void attach()}>선택 첨부</button>
      </fieldset>
      <small>첨부 8개 · 선택당 100줄/16 KiB · 전체 300줄/48 KiB. 초과하면 범위를 줄이거나 제외하세요.</small>
      <details><summary>다른 초안에서 선택 복사</summary>
        <p>대상: {target.hostId} · {target.worktree} · {target.threadId}</p>
        {Object.entries(all).filter(([k]) => k !== key).flatMap(([k, list]) => list.map(item =>
          <button key={k + item.attachmentId} type="button" disabled={disabled} onClick={() => copy(item)}>
            {item.origin.hostId} · {item.origin.worktree} · {item.origin.threadId}: {item.filePath}:{item.startLine}–{item.endLine} 복사
          </button>))}
      </details>
    </details>
    {files.map(item => <div key={item.attachmentId}>
      <label><input type="checkbox" checked={item.included} disabled={disabled}
        onChange={() => change(files.map(a => a === item ? { ...a, included: !a.included } : a))} />포함</label>
      <details><summary>{item.filePath}:{item.startLine}–{item.endLine} · 원래 선택 보기</summary>
        <code>{item.origin.hostId} · {item.worktreeId} · {item.snapshotId}</code>
        <pre>{item.before}{item.selected}{item.after}</pre></details>
      {item.staleReason && <p role="alert">{item.staleReason}
        <label><input type="checkbox" disabled={disabled} checked={item.useOriginal}
          onChange={() => change(files.map(a => a === item ? { ...a, useOriginal: !a.useOriginal } : a))} />이전 판 기준으로 보내기</label></p>}
      <button type="button" disabled={disabled} onClick={() => change(files.filter(a => a !== item))}>첨부 삭제</button>
    </div>)}
    {files.some(item => item.included) && <details><summary>보낼 정확한 내용 미리보기</summary>
      <pre data-testid="file-message-preview">{fileMessage(draft, files, target)}</pre></details>}
    {error && <p role="alert">{error}</p>}
  </section>;
}
