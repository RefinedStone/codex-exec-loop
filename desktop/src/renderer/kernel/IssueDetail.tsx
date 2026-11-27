import { useState } from "react";
import { isVaultRestoreTarget } from "../../shared/vaultRestore";
import { Markdown } from "../conversation/Markdown";
import { Icon } from "../design/icons";
import { IssueTriageView } from "./IssueTriageView";
import { IssueDecisionCards } from "./IssueDecisionCards";
import { IssueInterventions } from "./IssueInterventions";
import { IssueExecutionTracking } from "./IssueExecutionTracking";
import {
  EMPTY_ISSUE_DECISION_DRAFT, EMPTY_ISSUE_INTERVENTION_DRAFT,
  decisionDraftIdentity, interventionDraftIdentity, issueDraftIdentity,
  type IssueCommandOutcome, type IssueDecisionDraft, type IssueInterventionName,
} from "./issueInteractions";
import type { IssueDetailAnswer, IssueEventReference } from "./protocol";

interface IssueDetailProps {
  selectedIssue: IssueEventReference;
  detail: IssueDetailAnswer | null;
  loading: boolean;
  stale: boolean;
  error: string | null;
  busy: boolean;
  onAnswer(decisionId: string, option: string, note: string, criteria?: import("./protocol").TaskAcceptanceCriterion[]): Promise<void>;
  onIntervene(action: IssueInterventionName, message?: string, confirm?: boolean): Promise<IssueCommandOutcome>;
  onOpenConversation(): Promise<void>;
  onRefresh(): Promise<void>;
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

export function IssueDetail(props: IssueDetailProps) {
  return <SelectedIssueDetail key={issueDraftIdentity(props.selectedIssue)} {...props} />;
}

/** 현재 선택과 일치하는 최신 응답에서만 공유본을 읽는다. 저장된 판정의 stale 여부는 본문과 별개다. */
function SelectedIssueDetail({ selectedIssue, detail, loading, stale, error, busy, onAnswer, onIntervene, onOpenConversation, onRefresh, onBack, onOpenUrl }: IssueDetailProps) {
  const current = !stale && detail?.status === "ready" && sameIssue(selectedIssue, detail.item.issue) ? detail : null;
  // 재조회 중에는 서버의 과거 표시값을 숨기되 사용자 초안만 이슈 상세 수명에 보관한다.
  const [storedDrafts, setDrafts] = useState(() => ({
    basis: "",
    decisions: new Map<string, IssueDecisionDraft>(),
    interventionBasis: "",
    intervention: EMPTY_ISSUE_INTERVENTION_DRAFT,
  }));
  let drafts = storedDrafts;
  if (current) {
    const decisionKeys = (current.open_decisions ?? []).map(decisionDraftIdentity);
    const interventionBasis = interventionDraftIdentity(current);
    const basis = JSON.stringify([decisionKeys, interventionBasis]);
    if (basis !== drafts.basis) {
      drafts = {
        basis,
        decisions: new Map<string, IssueDecisionDraft>(decisionKeys.map((key) => [key, drafts.decisions.get(key) ?? EMPTY_ISSUE_DECISION_DRAFT])),
        interventionBasis,
        intervention: interventionBasis === drafts.interventionBasis ? drafts.intervention : EMPTY_ISSUE_INTERVENTION_DRAFT,
      };
      // 새 기준의 첫 표시부터 오래된 선택을 쓰지 않고, 사라진 결정의 초안도 폐기한다.
      setDrafts(drafts);
    }
  } else if (!stale && detail?.status === "not_found" && drafts.basis !== "") {
    drafts = { basis: "", decisions: new Map<string, IssueDecisionDraft>(), interventionBasis: "", intervention: EMPTY_ISSUE_INTERVENTION_DRAFT };
    setDrafts(drafts);
  }
  const [openingOriginal, setOpeningOriginal] = useState(false);
  const [originalError, setOriginalError] = useState<string | null>(null);
  const item = current?.item ?? null;
  const shared = item?.current.status === "observed" && item.current.content.kind === "shared" ? item.current.content.shared_copy : null;
  const candidate = item && shared && { workspace: item.issue.project_scope.workspace_dir, issueUrl: item.presentation.url, issue: item.issue };
  const restoreTarget = isVaultRestoreTarget(candidate) ? candidate : null;
  async function openOriginal(): Promise<void> {
    if (!restoreTarget || openingOriginal || busy || loading) return;
    setOpeningOriginal(true);
    setOriginalError(null);
    try { await window.akra.vaultRestore.open(restoreTarget); }
    catch { setOriginalError("로컬 복원본 창을 열 수 없습니다. 현재 이슈를 다시 확인해 주세요."); }
    finally { setOpeningOriginal(false); }
  }
  return (
    <article className="issues-detail" aria-label="이슈 상세" data-testid="issues-detail" aria-busy={loading}>
      <div className="issues-detail-toolbar">
        <button type="button" className="issues-back" onClick={onBack} data-testid="issues-detail-back"><Icon name="chevronLeft" size={14} />목록으로</button>
        <button type="button" className="issues-file-open" disabled={loading} onClick={() => void onRefresh()} data-testid="issue-execution-refresh"><Icon name="refresh" size={14} />다시 확인</button>
      </div>
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
            <IssueExecutionTracking execution={current.execution} feedback={current.feedback} timeline={current.timeline} onOpenUrl={onOpenUrl} />
            {Boolean(current.comment_requests?.length) && <section aria-label="기록된 댓글 요청" className="issues-detail-section">
              <h3>기록된 댓글 요청</h3>
              <p>이 이슈에서 기록한 요청입니다 (최대 100건). 댓글은 승인 영수증이 아니며 자동으로 적용하지 않습니다.</p>
              <ul>{current.comment_requests?.map(record => <li key={record.comment_id}>
                <code>/akra {record.kind}</code> · 작성자 {record.author_id} · 댓글 {record.comment_id}
              </li>)}</ul>
            </section>}
            <IssueDecisionCards decisions={current.open_decisions ?? []} history={current.decision_history} asOfUnixMs={current.as_of_unix_ms} busy={busy || loading} onAnswer={onAnswer} drafts={drafts.decisions} onDraftChange={(key, draft) => {
              setDrafts((previous) => {
                if (!previous.decisions.has(key)) return previous;
                const decisions = new Map(previous.decisions);
                decisions.set(key, draft);
                return { ...previous, decisions };
              });
            }} />
            <IssueInterventions detail={current} busy={busy || loading} onIntervene={onIntervene} onOpenConversation={onOpenConversation} draft={drafts.intervention} onDraftChange={(draft, submitted) => {
              // 늦은 성공은 같은 기준의 새 입력이나 다시 연 확인 폼도 덮지 않는다.
              const expected = drafts.interventionBasis;
              setDrafts((previous) => previous.interventionBasis === expected && (!submitted || previous.intervention === submitted)
                ? { ...previous, intervention: draft } : previous);
            }} />
          </>}
          <div className="issues-detail-columns">
            <section className="issues-detail-section" aria-label="현재 공유본">
              <h3>현재 공유본</h3>
              {restoreTarget && <button type="button" className="issues-back" disabled={busy || loading || openingOriginal} onClick={() => void openOriginal()} data-testid="issues-detail-original">원문 보기</button>}
              {originalError && <p className="issues-inline-notice error" role="alert">{originalError}</p>}
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
