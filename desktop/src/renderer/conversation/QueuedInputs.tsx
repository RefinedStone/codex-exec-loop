import { useState, useSyncExternalStore } from "react";
import type { NextTurnQueue } from "../session/nextTurnQueue";

export function QueuedInputs({ queue, threadId, resume }: { queue: NextTurnQueue; threadId: string; resume: () => void }) {
  const state = useSyncExternalStore(queue.subscribe, () => queue.snapshot(threadId));
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<{ id: string; text: string } | null>(null);
  if (!state.items.length) return null;
  const run = (action: () => void) => {
    setError(null);
    try { action(); } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
  };
  return <section className="queued-inputs" aria-label="다음 턴 예약" data-testid="queued-inputs">
    <div><strong>다음 턴 예약 {state.items.length}</strong> <span className="muted">이 창을 닫으면 지워집니다.</span></div>
    {state.paused && <p role="status">{state.paused} <button type="button" onClick={() => run(resume)} disabled={state.items.some((item) => item.state !== "queued")}>예약 계속</button></p>}
    {error && <p className="error">{error}</p>}
    <ol>{state.items.map((item, index) => <li key={item.id}>
      {editing?.id === item.id ? <>
        <textarea aria-label="예약 입력 수정" value={editing.text} onChange={(event) => setEditing({ id: item.id, text: event.target.value })} />
        <button type="button" onClick={() => run(() => { queue.edit(threadId, item.id, editing.text); setEditing(null); })}>수정 저장</button>
        <button type="button" onClick={() => setEditing(null)}>취소</button>
      </> : <><span className="queued-input-text">{item.text}</span>
        <span>{item.state === "sending" ? "접수 확인 중" : item.state === "unknown" ? "결과 불명" : "대기"}</span>
        <button type="button" disabled={item.state !== "queued"} onClick={() => setEditing({ id: item.id, text: item.text })}>수정</button>
        <button type="button" aria-label={`${index + 1}번 예약 위로`} disabled={item.state !== "queued" || index === 0} onClick={() => queue.move(threadId, item.id, -1)}>↑</button>
        <button type="button" aria-label={`${index + 1}번 예약 아래로`} disabled={item.state !== "queued" || index === state.items.length - 1} onClick={() => queue.move(threadId, item.id, 1)}>↓</button>
        <button type="button" disabled={item.state === "sending"} onClick={() => queue.remove(threadId, item.id)}>{item.state === "unknown" ? "기록 확인 후 목록에서 지우기" : "삭제"}</button>
      </>}
    </li>)}</ol>
  </section>;
}
