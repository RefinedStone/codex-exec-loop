import type { IssueTriageModeState, IssueTriageState } from "./protocol";

export const ISSUE_MODE_LABEL: Record<IssueTriageModeState, string> = {
  off: "끄기",
  observe: "관찰",
  propose: "제안",
  gated: "승인 후 진행",
  auto: "자동",
};

const ELIGIBILITY_LABEL = { eligible: "대상", ineligible: "제외", unknown: "미확인" } as const;
const READINESS_LABEL = { ready: "준비됨", needs_info: "정보 필요", unknown: "미확인" } as const;
const RISK_LABEL = { low: "낮음", normal: "보통", high: "높음" } as const;
const GATE_LABEL = { start: "시작", spec: "명세", plan: "계획", delivery: "결과 제출", merge: "병합", close: "이슈 닫기" } as const;
const ACTION_LABEL = { human: "사람 승인", timeout_default: "시간 만료 시 기본 처리", timeout_arbiter: "시간 만료 시 중재", auto: "자동" } as const;

/** 분류의 이름만 번역한다. 판정, 이유, 다음 행동과 관문 순서는 서버의 조회 결과다. */
export function IssueTriageView({ triage }: { triage: IssueTriageState | null | undefined }) {
  if (!triage) return (
    <section className="issues-detail-section" aria-label="판정">
      <h3>판정</h3>
      <p className="muted" data-testid="issues-triage-empty">아직 저장된 판정이 없습니다.</p>
    </section>
  );
  return (
    <section className="issues-detail-section" aria-label="판정" data-testid="issues-triage">
      <h3>판정</h3>
      {triage.stale && <p className="issues-inline-notice warning" role="status" data-testid="issues-triage-stale">현재 공유본에 맞춰 판정을 다시 확인해야 합니다.</p>}
      <dl className="issues-facts">
        <div><dt>대상 여부</dt><dd data-testid="issues-triage-eligibility">{ELIGIBILITY_LABEL[triage.eligibility]}</dd></div>
        <div><dt>준비 상태</dt><dd data-testid="issues-triage-readiness">{READINESS_LABEL[triage.readiness]}</dd></div>
        <div><dt>위험도</dt><dd data-testid="issues-triage-risk">{RISK_LABEL[triage.risk_class]}</dd></div>
        <div><dt>적용 모드</dt><dd data-testid="issues-triage-mode">{ISSUE_MODE_LABEL[triage.effective_mode]}</dd></div>
      </dl>
      <h4>이유와 다음 행동</h4>
      {triage.reasons.length === 0 ? <p className="muted">저장된 이유가 없습니다.</p> : (
        <ol className="issues-reasons">
          {triage.reasons.map((reason, index) => (
            <li key={index} data-testid="issues-triage-reason" data-reason={reason.code}>
              <p data-testid="issues-triage-summary">{reason.summary}</p>
              <p className="issues-next-summary" data-testid="issues-triage-next">{reason.next_summary}</p>
            </li>
          ))}
        </ol>
      )}
      <h4>관문</h4>
      {triage.gates.length === 0 ? <p className="muted">저장된 관문이 없습니다.</p> : (
        <ul className="issues-gates">
          {triage.gates.map((gate, index) => (
            <li key={index} data-testid="issues-triage-gate" data-gate={gate.gate} data-enabled={gate.enabled}>
              <strong>{GATE_LABEL[gate.gate]}</strong>
              <span>{ACTION_LABEL[gate.action]}</span>
              <span className="issues-gate-state">{gate.enabled ? "활성" : "비활성"}</span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
