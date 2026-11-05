// 열린 스레드의 대화 기록이다. 턴마다 아이템을 순서대로 그린다. 입력창은 다음 단계(P3 입력)에서 붙인다.
import { useDesktopStore } from "../session/desktopStore";
import { ItemView } from "./ItemView";

const TURN_STATUS_LABEL: Record<string, string> = {
  inProgress: "진행 중",
  completed: "완료",
  interrupted: "중단됨",
  failed: "실패",
};

export function ConversationView() {
  const openThreadId = useDesktopStore((state) => state.openThreadId);
  const transcript = useDesktopStore((state) => state.transcript);
  const transcriptError = useDesktopStore((state) => state.transcriptError);

  if (!openThreadId) {
    return <p className="muted empty">왼쪽에서 스레드를 고르세요.</p>;
  }
  if (transcriptError) {
    return <p className="error">{transcriptError}</p>;
  }
  if (!transcript) {
    return <p className="muted empty">불러오는 중…</p>;
  }
  return (
    <section className="conversation" data-testid="conversation">
      <header className="conversation-header">
        <h2>{transcript.title}</h2>
        <span className="muted">{transcript.cwd}</span>
      </header>
      {transcript.turns.map((turn) => (
        <article key={turn.id} className="turn" data-testid="turn" data-status={turn.status}>
          {turn.items.map((item) => (
            <ItemView key={item.id} item={item} />
          ))}
          {turn.status !== "completed" && (
            <div className={`turn-status status-${turn.status}`}>{TURN_STATUS_LABEL[turn.status] ?? turn.status}</div>
          )}
          {turn.errorMessage && <div className="error">{turn.errorMessage}</div>}
        </article>
      ))}
    </section>
  );
}
