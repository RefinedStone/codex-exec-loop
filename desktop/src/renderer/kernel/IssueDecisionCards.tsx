import type { IssueDecisionRequestState } from "./protocol";
import { EMPTY_ISSUE_DECISION_DRAFT, ISSUE_DECISION_KIND_LABEL, ISSUE_DECISION_STATE_LABEL, decisionDraftIdentity, issueTextBytes, type IssueDecisionDraft, type ReadyIssueDetail } from "./issueInteractions";

export interface IssueDecisionCardsProps {
  decisions: readonly IssueDecisionRequestState[];
  history: ReadyIssueDetail["decision_history"];
  busy: boolean;
  drafts: ReadonlyMap<string, IssueDecisionDraft>;
  onDraftChange(key: string, draft: IssueDecisionDraft): void;
  onAnswer(decisionId: string, option: string, note: string): Promise<void>;
}

/** 질문·선택지·권고는 현재 공개 정책을 통과한 서버 표시값만 사용한다. */
function DecisionCard({ request, busy, state = request.state, reason, onAnswer, draft = EMPTY_ISSUE_DECISION_DRAFT, onDraftChange }: {
  request: IssueDecisionRequestState;
  busy: boolean;
  state?: string;
  reason?: string;
  onAnswer: IssueDecisionCardsProps["onAnswer"];
  draft?: IssueDecisionDraft;
  onDraftChange?(draft: IssueDecisionDraft): void;
}) {
  const { option, note } = draft;
  const setOption = (option: string) => onDraftChange?.({ ...draft, option });
  const setNote = (note: string) => onDraftChange?.({ ...draft, note });
  const open = state === "open";
  const tooLong = issueTextBytes(note) > 4_096;
  const selected = request.options.some((value) => value.id === option);
  return (
    <section className="issues-decision-card" data-testid="issue-decision-card" data-decision-id={request.id} data-state={state} aria-label={ISSUE_DECISION_KIND_LABEL[request.kind] ?? "결정"}>
      <div className="issues-decision-heading">
        <strong>{ISSUE_DECISION_KIND_LABEL[request.kind] ?? "결정"}</strong>
        <span className="issues-status" data-status={state}>{ISSUE_DECISION_STATE_LABEL[state] ?? state}</span>
      </div>
      <p className="issues-decision-question" data-testid="issue-decision-question">{request.question}</p>
      {reason && <p className="issues-inline-notice muted" data-testid="issue-decision-reason">{reason}</p>}
      <fieldset disabled={busy || !open} className="issues-decision-options">
        <legend className="sr-only">답변 선택</legend>
        {request.options.map((choice) => (
          <label key={choice.id} className="issues-decision-option" data-recommended={request.recommended === choice.id}>
            <input type="radio" name={"issue-decision-" + request.id} value={choice.id} checked={option === choice.id} onChange={() => setOption(choice.id)} />
            <span>
              <span className="issues-decision-option-label">{choice.label}{request.recommended === choice.id && <span className="issues-recommended">권고</span>}</span>
              <span className="issues-decision-effect">{choice.effect}</span>
            </span>
          </label>
        ))}
      </fieldset>
      {open && (
        <>
          <label className="issues-input-label">
            공유할 추가 설명 <span className="muted">(선택)</span>
            <textarea value={note} onChange={(event) => setNote(event.target.value)} disabled={busy} rows={2} maxLength={4_096} aria-invalid={tooLong} placeholder="선택에 필요한 설명을 적어 주세요." data-testid="issue-decision-note" />
          </label>
          {tooLong && <p className="issues-inline-notice error" role="alert">추가 설명은 4,096바이트 이내로 적어 주세요.</p>}
          <div className="issues-decision-footer">
            <span>시스템 확인 창에서 한 번 더 확인합니다.</span>
            <button type="button" className="issues-action-button primary" disabled={busy || !selected || tooLong} onClick={() => void onAnswer(request.id, option, note)} data-testid="issue-decision-answer">{busy ? "처리 중…" : "선택 확인"}</button>
          </div>
        </>
      )}
    </section>
  );
}

export function IssueDecisionCards({ decisions, history, busy, onAnswer, drafts, onDraftChange }: IssueDecisionCardsProps) {
  const openIds = new Set(decisions.map((request) => request.id));
  const resolved = (history ?? []).filter((entry) => !openIds.has(entry.decision.decision_id));
  return (
    <div className="issues-decisions" data-testid="issue-decisions">
      {decisions.map((request) => {
        const key = decisionDraftIdentity(request);
        return <DecisionCard key={key} request={request} busy={busy} onAnswer={onAnswer} draft={drafts.get(key)} onDraftChange={(draft) => onDraftChange(key, draft)} />;
      })}
      {resolved.length > 0 && (
        <section className="issues-decision-history" aria-label="이전 결정" data-testid="issue-decision-history">
          <h3>이전 결정</h3>
          {resolved.map((entry) => entry.request ? (
            <DecisionCard key={entry.decision.decision_id} request={entry.request} state={entry.decision.state} busy={true} reason={entry.resolution_reason?.summary ?? "완료된 결정은 다시 답할 수 없습니다."} onAnswer={onAnswer} />
          ) : (
            <article className="issues-decision-card" key={entry.decision.decision_id} data-testid="issue-decision-history-item" data-state={entry.decision.state}>
              <div className="issues-decision-heading">
                <strong>{ISSUE_DECISION_KIND_LABEL[entry.decision.kind] ?? "이전 결정"}</strong>
                <span className="issues-status">{ISSUE_DECISION_STATE_LABEL[entry.decision.state] ?? entry.decision.state}</span>
              </div>
              <p className="issues-inline-notice muted">{entry.resolution_reason?.summary ?? "완료된 결정입니다."}</p>
              <p className="issues-inline-notice muted">이전 질문과 선택지는 현재 공개 범위에서 표시할 수 없습니다.</p>
            </article>
          ))}
        </section>
      )}
    </div>
  );
}
