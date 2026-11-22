import { Markdown } from "../conversation/Markdown";
import { Icon } from "../design/icons";
import { IssueTriageView } from "./IssueTriageView";
import { IssueDecisionCards } from "./IssueDecisionCards";
import { IssueInterventions } from "./IssueInterventions";
import type { IssueInterventionName } from "./issueInteractions";
import type { IssueDetailAnswer, IssueEventReference } from "./protocol";

interface IssueDetailProps {
  selectedIssue: IssueEventReference;
  detail: IssueDetailAnswer | null;
  loading: boolean;
  stale: boolean;
  error: string | null;
  busy: boolean;
  onAnswer(decisionId: string, option: string, note: string): Promise<void>;
  onIntervene(action: IssueInterventionName, message?: string, confirm?: boolean): Promise<void>;
  onOpenConversation(): Promise<void>;
  onBack(): void;
  onOpenUrl(url: string): void;
}

function sameIssue(left: IssueEventReference, right: IssueEventReference): boolean {
  return left.provider === right.provider
    && left.server_id === right.server_id
    && left.repository_id === right.repository_id
    && left.issue_id === right.issue_id
    && left.project_scope.workspace_dir === right.project_scope.workspace_dir;
}

/** 현재 선택과 일치하는 최신 응답에서만 공유본을 읽는다. 저장된 판정의 stale 여부는 본문과 별개다. */
export function IssueDetail({ selectedIssue, detail, loading, stale, error, busy, onAnswer, onIntervene, onOpenConversation, onBack, onOpenUrl }: IssueDetailProps) {
  const current = !stale && detail?.status === "ready" && sameIssue(selectedIssue, detail.item.issue) ? detail : null;
  const item = current?.item ?? null;
  const shared = item?.current.status === "observed" && item.current.content.kind === "shared" ? item.current.content.shared_copy : null;
  return (
    <article className="issues-detail" aria-label="이슈 상세" data-testid="issues-detail" aria-busy={loading}>
      <button type="button" className="issues-back" onClick={onBack} data-testid="issues-detail-back"><Icon name="chevronLeft" size={14} />목록으로</button>
      {error && <p className="issues-inline-notice error" role="alert" data-testid="issues-detail-error">{error}</p>}
      {loading && <p className="issues-inline-notice muted" role="status" data-testid="issues-detail-loading">현재 이슈 상세를 확인하고 있습니다.</p>}
      {!loading && stale && !error && <p className="issues-inline-notice muted" role="status">새로 고침으로 현재 이슈 상세를 확인하세요.</p>}
      {!loading && !stale && detail?.status === "not_found" && <p className="issues-inline-notice muted" role="status">현재 저장소에서 이 이슈를 찾을 수 없습니다.</p>}
      {!loading && !stale && detail?.status === "unavailable" && <p className="issues-inline-notice error" role="alert">이슈 상세를 읽을 수 없습니다. 새로 고침으로 다시 확인하세요.</p>}
      {item && (
        <>
          <header className="issues-detail-heading">
            <div className="issues-row-heading">
              <span className="issues-number">{item.presentation.number == null ? "번호 미확인" : `#${item.presentation.number}`}</span>
              <span className="issues-status" data-status={item.presentation.status.code}>{item.presentation.status.label}</span>
              {item.presentation.url && <button type="button" className="icon-button issues-external" title="브라우저에서 열기" aria-label="브라우저에서 열기" onClick={() => onOpenUrl(item.presentation.url!)} data-testid="issues-detail-external"><Icon name="popOut" size={14} /></button>}
            </div>
            <h2 data-testid="issues-detail-title">{item.presentation.title ?? "제목을 확인할 수 없습니다."}</h2>
            <p className="issues-detail-reason">{item.presentation.reason.summary}</p>
          </header>
          {current && <>
            <IssueDecisionCards decisions={current.open_decisions ?? []} history={current.decision_history} busy={busy} onAnswer={onAnswer} />
            <IssueInterventions key={JSON.stringify(selectedIssue)} detail={current} busy={busy} onIntervene={onIntervene} onOpenConversation={onOpenConversation} />
          </>}
          <div className="issues-detail-columns">
            <section className="issues-detail-section" aria-label="현재 공유본">
              <h3>현재 공유본</h3>
              {shared ? (
                <div className="issues-shared-copy" data-testid="issues-shared-copy">
                  {shared.body ? <Markdown text={shared.body} /> : <p className="muted">공유된 본문이 비어 있습니다.</p>}
                </div>
              ) : (
                <p className="issues-inline-notice muted" data-testid="issues-shared-unavailable">
                  {item.current.status === "unknown" ? "현재 공유본을 확인하지 못했습니다." : item.current.status === "not_yet_observed" ? "아직 현재 공유본을 관찰하지 않았습니다." : "현재 정책에서 공유하지 않은 본문입니다."}
                </p>
              )}
            </section>
            <IssueTriageView triage={item.triage} />
          </div>
        </>
      )}
    </article>
  );
}
