import { useState, useSyncExternalStore } from "react";
import { useDesktopStore } from "../session/desktopStore";
import type { ThreadCompactionController } from "../session/threadCompaction";
import "./threadCompaction.css";

export function ThreadCompactionControl({ controller }: { controller: ThreadCompactionController }) {
  const threadId = useDesktopStore(state => state.openThreadId);
  // 선택, 접속, 읽기 잠금, 턴 변화를 따라 버튼의 가용성을 다시 계산한다.
  useDesktopStore(state => state.connection);
  useDesktopStore(state => state.writerElsewhere);
  useDesktopStore(state => state.transcript);
  const state = useSyncExternalStore(controller.subscribe, () => controller.snapshot(threadId));
  const [failure, setFailure] = useState<string | null>(null);
  if (!threadId) return null;
  const unavailable = controller.unavailableReason(threadId);
  const submit = () => {
    setFailure(null);
    try { controller.start(threadId); } catch (error) { setFailure(error instanceof Error ? error.message : "압축을 시작하지 못했습니다."); }
  };
  const refresh = () => {
    setFailure(null);
    void controller.refresh(threadId).catch(error => setFailure(error instanceof Error ? error.message : "압축 결과를 확인하지 못했습니다."));
  };
  return <div className="thread-compaction" data-testid="thread-compaction">
    <div className="thread-compaction-actions">
      <button type="button" className="chip-button" data-testid="compact-start" disabled={unavailable !== null} title={unavailable ?? "엔진이 현재 대화의 문맥을 압축합니다. 모델 사용량이 발생할 수 있습니다."} onClick={submit}>문맥 압축</button>
      {state.phase === "unknown" && <button type="button" className="chip-button" data-testid="compact-refresh" disabled={state.checking} onClick={refresh}>{state.checking ? "확인하는 중…" : "상태 다시 확인"}</button>}
      <span className="muted">엔진으로 문맥을 줄입니다. 모델 사용량이 발생할 수 있습니다.</span>
    </div>
    {(state.message || failure) && <p role={failure || state.phase === "failed" ? "alert" : "status"} data-testid="compact-status" data-phase={state.phase}>{failure ?? state.message}</p>}
  </div>;
}
