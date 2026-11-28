import { useState } from "react";
import { DiffFeedbackEditor } from "./DiffFeedbackSelection";
import "./fileAttachments.css";
import { useUiStore, setFileList } from "../session/uiStore";
import { readSource, type ManifestRequest } from "./attachmentSource";
import { captureSelection, fileMessage, targetKey, type DraftTarget, type FileAttachment } from "./fileAttachment";
import type { ManifestLedger } from "./attachmentManifest";

function LedgerDetails({ label, ledger }: { readonly label: string; readonly ledger: ManifestLedger }) {
  return <details data-testid="attachment-ledger"><summary>{label} · 버전 미확인 {ledger.unverifiedCount}</summary>
    <code>{ledger.id}</code>
    <p>장부에서 관측한 판입니다. 외부 최신성 또는 읽음을 확인하지 않습니다.</p>
    <ul>{ledger.entries.map(entry => <li key={JSON.stringify([entry.kind, entry.key])}>
      {entry.kind} · {entry.key} · {entry.selected ? "선택" : "제외"} ({entry.reason}) · {entry.observedVersion ?? "버전 없음"}
    </li>)}</ul>
  </details>;
}

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
      const { diff, manifestLedger } = await readSource(request, target, { path });
      add(await captureSelection(diff, { startLine: Number(start), endLine: Number(end) },
        { target, turnId, manifestId: manifestLedger.id, manifestLedger, draftRevision: Date.now() }));
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  };
  const copy = (item: FileAttachment) => {
    try { add({ ...item, attachmentId: crypto.randomUUID(), draftRevision: Date.now(), included: true,
      staleReason: "다른 대상에 복사한 선택", useOriginal: false }); setError(""); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
  };
  return <section aria-label="파일 첨부" className="file-attachments">
    <DiffFeedbackEditor target={target} request={request} disabled={disabled} />
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
    {files.map((item, index) => <div key={item.attachmentId} className="file-attachment-card" data-testid="file-attachment-card">
      <label><input type="checkbox" checked={item.included} disabled={disabled}
        onChange={() => change(files.map(a => a === item ? { ...a, included: !a.included } : a))} />포함</label>
      {!item.included && <small>요청에서 제외</small>}
      {item.manifestLedger ? <LedgerDetails label="원래 선택 장부" ledger={item.manifestLedger} />
        : <small>원래 장부의 버전 확인 상태를 알 수 없음</small>}
      {item.currentLedger && <LedgerDetails label="변경된 장부" ledger={item.currentLedger} />}
      <details><summary>{item.filePath}:{item.startLine}–{item.endLine} · {item.side === "old" ? "이전" : "이후"} · 원래 선택 보기</summary>
        <code>{item.sourceKind} · {item.oldPath ?? item.filePath} → {item.filePath}</code>
        <code>{item.origin.hostId} · {item.worktreeId} · {item.snapshotId}</code>
        <pre>{item.before}<mark>{item.selected}</mark>{item.after}</pre></details>
      {item.sourceKind !== "worktree_file" && <label>피드백 수정<textarea disabled={disabled} value={item.feedback}
        onChange={e => change(files.map(a => a === item ? { ...a, feedback: e.target.value, draftRevision: Date.now() } : a))} /></label>}
      {files.length > 1 && ([-1, 1] as const).map(step => <button key={step} type="button"
        disabled={disabled || index + step < 0 || index + step >= files.length} onClick={() => {
          const reordered = files.filter(a => a !== item);
          reordered.splice(index + step, 0, item);
          change(reordered);
        }}>{step < 0 ? "위로" : "아래로"}</button>)}
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
