import { ISSUE_MODE_LABEL } from "./IssueTriageView";
import type { IssueTriageObserveReportState } from "./protocol";

const COUNTS = [
  ["total", "전체 추적"],
  ["blocked", "진행 차단"],
  ["deferred", "보류"],
  ["needs_info", "정보 필요"],
  ["unknown", "미확인"],
  ["untriaged", "미판정"],
  ["stale", "판정 갱신 필요"],
  ["observation_only", "관찰 전용"],
] as const;

/** 전체 binding 보고를 그대로 표시한다. 현재 페이지 항목으로 합산하지 않는다. */
export function IssueObserveReport({ report }: { report: IssueTriageObserveReportState | null | undefined }) {
  return (
    <section className="issues-report" aria-label="관찰 보고" data-testid="issues-report">
      <h2>관찰 보고</h2>
      <p className="issues-section-description" data-testid="issues-report-scope">현재 저장소 연결에서 추적하는 전체 이슈 · 모든 페이지</p>
      {!report ? <p className="issues-inline-notice muted" role="status">현재 관찰 보고를 확인할 수 없습니다. 새로 고침으로 다시 확인하세요.</p> : (
        <>
          {report.binding_id && <p className="issues-report-binding"><span>연결</span><code data-testid="issues-report-binding">{report.binding_id}</code></p>}
          <p className="issues-report-assumption">{report.assumed_mode ? `가정 모드: ${ISSUE_MODE_LABEL[report.assumed_mode]}` : "가정 모드 미확인"} · 저장된 판정 기준</p>
          {report.policy_blocked && <p className="issues-inline-notice warning" role="status" data-testid="issues-report-policy-blocked">정책을 확인해야 합니다. 정책 보기에서 원인과 파일 위치를 확인하세요.</p>}
          <dl className="issues-report-primary">
            <div><dt>자동 시작</dt><dd data-testid="issues-report-automatic-start">{report.automatic_start}</dd></div>
            <div><dt>사람 승인</dt><dd data-testid="issues-report-human-approval">{report.human_approval}</dd></div>
            <div><dt>제외</dt><dd data-testid="issues-report-excluded">{report.excluded}</dd></div>
          </dl>
          <dl className="issues-report-counts">
            {COUNTS.map(([key, label]) => <div key={key}><dt>{label}</dt><dd data-testid={`issues-report-${key}`}>{report[key]}</dd></div>)}
          </dl>
        </>
      )}
    </section>
  );
}
