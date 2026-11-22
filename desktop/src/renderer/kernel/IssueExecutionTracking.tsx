import { Icon } from "../design/icons";
import type { ReadyIssueDetail } from "./issueInteractions";

interface IssueExecutionTrackingProps {
  execution: ReadyIssueDetail["execution"];
  feedback: ReadyIssueDetail["feedback"];
  timeline: ReadyIssueDetail["timeline"];
  onOpenUrl(url: string): void;
}

const EXTERNAL_STATE_LABEL = {
  open: "외부 이슈 열림",
  closed: "외부 이슈 닫힘",
  unknown: "외부 이슈 상태 모름",
} as const;

const EXECUTION_UNAVAILABLE_LABEL = {
  read_failed: "실행 기록을 읽지 못했습니다.",
  work_id_mismatch: "현재 작업과 실행 기록이 일치하지 않습니다.",
} as const;

function timelineTime(value: string): string {
  const time = new Date(value);
  return Number.isNaN(time.valueOf()) ? "시각 미확인"
    : new Intl.DateTimeFormat("ko-KR", { dateStyle: "medium", timeStyle: "short" }).format(time);
}

/** 저장된 서버 표시값을 그린다. 상태·재시도·복구 행동을 횟수나 이벤트에서 추정하지 않는다. */
export function IssueExecutionTracking({ execution, feedback, timeline, onOpenUrl }: IssueExecutionTrackingProps) {
  const current = execution?.status === "ready" ? execution : null;
  const integration = feedback?.integration;
  return (
    <section className="issues-execution" aria-label="실행 추적" data-testid="issue-execution">
      <div className="issues-execution-heading">
        <h3>실행 추적</h3>
      </div>
      <p className="issues-section-description">다시 확인하면 현재 상태를 읽습니다.</p>
      {current ? (
        <>
          <p className="issues-execution-state" data-testid="issue-execution-state" data-state={current.state}>{current.state_label}</p>
          <dl className="issues-execution-counts">
            <div>
              <dt>실행 시도</dt>
              <dd data-testid="issue-execution-attempts">
                {current.execution_attempt_limit === null
                  ? current.execution_attempts + "회 · 상한 미확인"
                  : current.execution_attempts + " / " + current.execution_attempt_limit}
              </dd>
            </div>
            <div><dt>검증 시도</dt><dd data-testid="issue-verification-attempts">{current.verification_attempts}회</dd></div>
            {feedback && <div><dt>게시 시도</dt><dd data-testid="issue-publication-attempts">{feedback.publication_attempts}회</dd></div>}
          </dl>
          {current.review_pull_request_url && (
            <button type="button" className="issues-file-open issues-execution-link" onClick={() => onOpenUrl(current.review_pull_request_url!)} data-testid="issue-execution-pr">
              <Icon name="popOut" size={14} />PR 열기
            </button>
          )}
          {current.recovery && (
            <div className="issues-execution-recovery" data-testid="issue-execution-recovery" data-reason={current.recovery.reason}>
              <p data-testid="issue-execution-recovery-summary">{current.recovery.summary}</p>
              <p data-testid="issue-execution-next">{current.recovery.next_summary}</p>
              {current.recovery.requires_human && <span className="muted">사람의 확인이 필요합니다.</span>}
            </div>
          )}
        </>
      ) : execution?.status === "unavailable" ? (
        <p className="issues-inline-notice error" data-testid="issue-execution-unavailable">{EXECUTION_UNAVAILABLE_LABEL[execution.reason]}</p>
      ) : (
        <p className="issues-section-description" data-testid="issue-execution-empty">아직 저장된 실행 기록이 없습니다.</p>
      )}
      {feedback && (
        <div className="issues-execution-outcome" data-testid="issue-execution-outcome">
          {!current && <p data-testid="issue-publication-attempts">게시 시도 {feedback.publication_attempts}회</p>}
          {integration && (
            <div className="issues-execution-integration">
              <p data-testid="issue-integration-state">통합 완료</p>
              {integration.pull_request_url && integration.pull_request_url !== current?.review_pull_request_url && (
                <button type="button" className="issues-file-open issues-execution-link" onClick={() => onOpenUrl(integration.pull_request_url!)} data-testid="issue-integration-pr">
                  <Icon name="popOut" size={14} />통합 PR 열기
                </button>
              )}
            </div>
          )}
          {feedback.external_state && <p data-testid="issue-external-state" data-state={feedback.external_state}>{EXTERNAL_STATE_LABEL[feedback.external_state]}</p>}
          {feedback.reason === "feedback_unconfirmed" && (
            <p className="issues-inline-notice" data-testid="issue-feedback-unconfirmed">게시 결과 확인 필요</p>
          )}
        </div>
      )}
      <section className="issues-execution-timeline" aria-label="최근 실행 기록" data-testid="issue-execution-timeline">
        <h4>최근 기록</h4>
        {timeline?.status === "ready" ? (
          <>
            {timeline.entries.length > 0 ? (
              <ol>
                {timeline.entries.map((entry) => (
                  <li key={entry.sequence} data-testid="issue-timeline-entry" data-sequence={entry.sequence} data-kind={entry.kind}>
                    <time dateTime={entry.recorded_at} title={entry.recorded_at}>{timelineTime(entry.recorded_at)}</time>
                    <p>{entry.summary}</p>
                  </li>
                ))}
              </ol>
            ) : <p className="issues-section-description">아직 기록이 없습니다.</p>}
            {timeline.has_older && <p className="issues-section-description" data-testid="issue-timeline-older">최근 기록을 최대 20개까지 표시합니다. 이전 기록은 이 화면에 표시되지 않습니다.</p>}
          </>
        ) : timeline?.status === "unavailable" ? (
          <p className="issues-inline-notice error" data-testid="issue-timeline-unavailable">최근 기록을 읽지 못했습니다. 다시 확인해 주세요.</p>
        ) : <p className="issues-section-description">최근 기록을 아직 제공하지 않습니다.</p>}
      </section>
    </section>
  );
}
