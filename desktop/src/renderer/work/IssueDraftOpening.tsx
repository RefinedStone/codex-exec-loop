export function IssueDraftOpening({name, state, onClose}: {readonly name: string; readonly state: {phase: "loading"} | {phase: "failed"; message: string}; onClose(): void}) {
    return (
      <aside className="work-panel issue-draft" data-testid="issue-draft-panel">
        <div className="work-header">
          <strong>이슈 수정 {name}</strong>
          <button type="button" onClick={onClose}>
            닫기
          </button>
        </div>
        {state.phase === "loading" && <p className="muted work-section">읽는 중…</p>}
        {state.phase === "failed" && (
          <div className="error work-section" data-testid="issue-draft-error">
            이슈를 읽을 수 없습니다. gh 로그인과 이슈 정책 파일을 확인하세요. {state.message}
          </div>
        )}
      </aside>
    );
}
