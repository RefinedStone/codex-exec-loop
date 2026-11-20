import { issueReadinessDisplay, validationErrorText, type IssueDraftReadiness } from "./issueDraft";

export function IssueReadinessPanel({
  result, pending, labels, disabled, onLabelsChange, onRefresh,
}: {
  result: IssueDraftReadiness | undefined;
  pending: boolean;
  labels: string;
  disabled: boolean;
  onLabelsChange(value: string): void;
  onRefresh(): void;
}) {
  const display = issueReadinessDisplay(result, pending);
  return (
    <section className="work-section issue-draft-readiness" data-testid="issue-draft-readiness">
      <h3>자동화 준비 검사</h3>
      <p className="muted">예상 라벨로 AI 전달 공유본을 검사합니다. 라벨을 붙이거나 작업을 승인하지 않습니다.</p>
      <label className="work-form">
        예상 라벨 (쉼표로 구분)
        <input value={labels} onChange={(event) => onLabelsChange(event.target.value)}
          disabled={disabled} maxLength={4096} placeholder="예: akra:auto, docs"
          data-testid="issue-draft-expected-labels" />
      </label>
      <button type="button" disabled={disabled || pending} onClick={onRefresh}
        data-testid="issue-draft-readiness-refresh">준비검사 새로 고침</button>
      {display.status === "pending" ? <p className="muted" role="status" data-testid="issue-draft-readiness-pending">현재 초안을 검사하는 중…</p>
        : display.status === "unavailable" ? <p className="muted">준비검사 결과가 없습니다. 미리보기를 새로 고쳐 주세요.</p>
        : display.status === "cannot_assess" ? (
          <div className="issue-draft-warning" role="status" data-testid="issue-draft-readiness-blocked" data-reason={display.reason}>
            <strong>{display.summary}</strong>
            <p>{display.nextSummary}</p>
          </div>
        ) : (
          <div data-testid="issue-draft-readiness-assessed" data-readiness={display.assessment.readiness}>
            <strong>{display.assessment.summary}</strong>
            <p data-testid="issue-draft-readiness-risk">예상 위험: {display.assessment.riskSummary}</p>
            <p className="muted">{display.assessment.modeSummary}</p>
            {display.assessment.missingSections.length > 0 && (
              <div data-testid="issue-draft-readiness-missing">
                <h4>부족한 섹션</h4>
                <ul>{display.assessment.missingSections.map((section) => <li key={section}>{section}</li>)}</ul>
              </div>
            )}
            {display.assessment.errors.length > 0 && (
              <ul className="issue-draft-warning" data-testid="issue-draft-readiness-errors">
                {display.assessment.errors.map((error, index) => <li key={index}>{validationErrorText(error)}</li>)}
              </ul>
            )}
            <ul data-testid="issue-draft-readiness-reasons">
              {display.assessment.reasons.map((reason, index) => <li key={index}>{reason.summary}</li>)}
            </ul>
            <h4>예상 관문</h4>
            <p className="muted">게시 후 선택·신뢰와 현재 이슈를 다시 확인합니다. 아래 값은 실행 허가가 아닙니다.</p>
            <table data-testid="issue-draft-readiness-gates">
              <thead><tr><th>관문</th><th>예상 처리</th></tr></thead>
              <tbody>{display.assessment.expectedGates.map((gate, index) => (
                <tr key={index}><td>{gate.summary}</td><td>{gate.actionSummary}</td></tr>
              ))}</tbody>
            </table>
          </div>
        )}
    </section>
  );
}
