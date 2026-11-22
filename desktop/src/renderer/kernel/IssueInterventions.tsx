import { useState } from "react";
import { ISSUE_ACTION_LABEL, issueTextBytes, type IssueInterventionName, type ReadyIssueDetail } from "./issueInteractions";

export function IssueInterventions({ detail, busy, onIntervene, onOpenConversation }: {
  detail: ReadyIssueDetail;
  busy: boolean;
  onIntervene(action: IssueInterventionName, message?: string, confirm?: boolean): Promise<void>;
  onOpenConversation(): Promise<void>;
}) {
  const [message, setMessage] = useState("");
  const [steering, setSteering] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const actions = detail.actions ?? [];
  const control = detail.control;
  const steer = actions.find((action) => action.command === "steer_issue");
  const cancel = actions.find((action) => action.command === "cancel_issue_work");
  const conversation = detail.conversation;
  const tooLong = issueTextBytes(message) > 65_536;
  const act = (action: IssueInterventionName) => {
    if (action === "steer_issue") { setSteering((value) => !value); setCancelling(false); return; }
    if (action === "cancel_issue_work") { setCancelling(true); setSteering(false); return; }
    void onIntervene(action);
  };
  return (
    <section className="issues-interventions" aria-label="이슈 개입" data-testid="issue-interventions">
      <div className="issues-intervention-heading">
        <h3>작업 제어</h3>
        {control && <span className="issues-control-state" data-testid="issue-control-state">{control.cancelled ? "작업 취소됨" : control.taken_over ? "사용자가 맡음" : control.held ? "보류 중" : !control.tracked ? "추적 중지됨" : "추적 중"}</span>}
      </div>
      <div className="issues-action-list">
        {actions.map((action) => <button type="button" className={"issues-action-button" + (action.command === "cancel_issue_work" ? " destructive" : "")} key={action.command} disabled={busy || !action.enabled} title={action.reason ?? undefined} onClick={() => act(action.command)} data-testid={"issue-action-" + action.command}>{ISSUE_ACTION_LABEL[action.command]}</button>)}
        <button type="button" className="issues-action-button" disabled={busy || !conversation || conversation.kind === "unavailable"} title={conversation?.kind === "unavailable" ? conversation.reason : undefined} onClick={() => void onOpenConversation()} data-testid="issue-open-conversation">내 대화로 열기</button>
      </div>
      {actions.filter((action) => !action.enabled && action.reason).map((action) => <p key={action.command} className="issues-action-reason" data-testid="issue-action-reason">{ISSUE_ACTION_LABEL[action.command]}: {action.reason}</p>)}
      {steering && steer?.enabled && (
        <form className="issues-steer" onSubmit={(event) => {
          event.preventDefault();
          if (!message.trim() || tooLong || busy) return;
          void onIntervene("steer_issue", message);
          setSteering(false);
          setMessage("");
        }}>
          <label className="issues-input-label">작업자에게 보낼 추가 지시<textarea value={message} onChange={(event) => setMessage(event.target.value)} rows={3} disabled={busy} maxLength={65_536} aria-invalid={tooLong} data-testid="issue-steer-message" /></label>
          {tooLong && <p className="issues-inline-notice error" role="alert">추가 지시는 65,536바이트 이내로 적어 주세요.</p>}
          <div className="issues-form-actions">
            <button type="button" className="issues-action-button" disabled={busy} onClick={() => setSteering(false)}>닫기</button>
            <button type="submit" className="issues-action-button primary" disabled={busy || !message.trim() || tooLong} data-testid="issue-steer-send">추가 지시 보내기</button>
          </div>
        </form>
      )}
      {cancelling && cancel?.enabled && (
        <div className="issues-cancel-confirmation" role="group" aria-label="작업 취소 확인">
          <p>이 이슈의 자동 작업을 취소하고 실행 허가를 회수합니다. 지금 취소할까요?</p>
          <div className="issues-form-actions">
            <button type="button" className="issues-action-button" disabled={busy} onClick={() => setCancelling(false)}>돌아가기</button>
            <button type="button" className="issues-action-button destructive" disabled={busy} onClick={() => { setCancelling(false); void onIntervene("cancel_issue_work", undefined, true); }} data-testid="issue-cancel-confirm">작업 취소 확인</button>
          </div>
        </div>
      )}
    </section>
  );
}
