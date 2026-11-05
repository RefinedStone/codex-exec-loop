// 열린 스레드의 승인 요청이다 (P3 §6.4). 다른 창에서 답하면 `serverRequest/resolved` 로 사라진다.
import { useDesktopStore } from "../session/desktopStore";
import { APPROVAL_DECISIONS, type ApprovalDecision, type PendingApproval } from "./approvals";

const DECISION_LABEL: Record<ApprovalDecision, string> = {
  accept: "허용",
  acceptForSession: "이 세션 동안 허용",
  decline: "거부",
};

export function ApprovalPrompt({
  onAnswer,
}: {
  onAnswer(requestId: PendingApproval["requestId"], decision: ApprovalDecision): void;
}) {
  const openThreadId = useDesktopStore((state) => state.openThreadId);
  const pending = useDesktopStore((state) => state.pendingApprovals);
  const approvals = pending.filter((approval) => approval.threadId === openThreadId);
  if (approvals.length === 0) {
    return null;
  }
  return (
    <section className="approvals" data-testid="approvals">
      {approvals.map((approval) => (
        <div key={String(approval.requestId)} className="approval" data-testid="approval">
          <div className="approval-title">
            {approval.kind === "command" ? "명령 실행 승인" : "파일 변경 승인"}
            {approval.command && <code>{approval.command}</code>}
          </div>
          {approval.cwd && <div className="muted">{approval.cwd}</div>}
          {approval.reason && <div>{approval.reason}</div>}
          <div className="approval-buttons">
            {APPROVAL_DECISIONS.map((decision) => (
              <button
                key={decision}
                type="button"
                onClick={() => onAnswer(approval.requestId, decision)}
                data-testid={`approval-${decision}`}
              >
                {DECISION_LABEL[decision]}
              </button>
            ))}
          </div>
        </div>
      ))}
    </section>
  );
}
