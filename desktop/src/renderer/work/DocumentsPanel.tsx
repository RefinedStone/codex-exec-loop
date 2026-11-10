// 열린 스레드의 관련 문서 패널이다 (plan 22). 스레드에 나온 Notion, Jira, GitHub 문서를 종류별로 보여 준다.
// 목록은 App 이 스레드를 열 때와 턴이 끝날 때 읽는다. 링크는 기본 브라우저로 열린다 (main 의 창 열기 처리).
import { groupByKind, kindLabel, sourceLabel, type ThreadDocumentsState } from "./threadDocuments";

export function DocumentsPanel({ state, onRefresh }: { state: ThreadDocumentsState; onRefresh(): void }) {
  return (
    <aside className="work-panel" data-testid="documents-panel">
      <div className="work-header">
        <strong>관련 문서</strong>
        <span className="muted">이 스레드에 나온 문서</span>
        <span className="work-actions">
          <button type="button" onClick={onRefresh} disabled={state.phase === "idle" || state.phase === "loading"} data-testid="documents-refresh">
            {state.phase === "loading" ? "읽는 중…" : "새로 고침"}
          </button>
        </span>
      </div>
      {state.phase === "idle" && <p className="muted work-section">스레드를 열면 그 스레드에 나온 문서를 보여 줍니다.</p>}
      {state.phase === "failed" && (
        <div className="error work-section" data-testid="documents-error">
          {state.message}
        </div>
      )}
      {state.phase === "loaded" && (
        <div className="work-scroll">
          {state.documents.length === 0 && <p className="muted work-section">이 스레드에는 아직 Notion, Jira, GitHub 문서가 나오지 않았습니다.</p>}
          {groupByKind(state.documents).map((group) => (
            <section key={group.kind} className="work-section" data-testid="documents-group">
              <h3>
                {kindLabel(group.kind)} ({group.documents.length})
              </h3>
              <ul>
                {group.documents.map((document) => (
                  <li key={`${document.kind}:${document.key}`} className="work-item" data-testid="documents-item">
                    <a className="work-link" href={document.url} target="_blank" rel="noreferrer" title={document.url}>
                      {document.title}
                    </a>
                    <div className="work-meta">
                      <span className="muted">처음 언급: {sourceLabel(document.source)}</span>
                    </div>
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </div>
      )}
    </aside>
  );
}
