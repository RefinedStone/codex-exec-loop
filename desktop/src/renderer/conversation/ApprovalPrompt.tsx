// 열린 스레드의 승인 요청이다 (P3 §6.4). 다른 창에서 답하면 `serverRequest/resolved` 로 사라진다.
// 파일 변경 승인은 요청에 변경 내용이 없으므로, 같은 itemId 의 `fileChange` 아이템에서 diff 를 찾아 보여 준다 (R1).
import { useMemo } from "react";

import { useDesktopStore } from "../session/desktopStore";
import { APPROVAL_DECISIONS, type ApprovalDecision, type PendingApproval } from "./approvals";
import { diffFileFromChange } from "./diff";
import { DiffFileList } from "./DiffView";
import type { Transcript } from "./transcript";

const APPROVAL_TITLE: Record<PendingApproval["kind"], string> = {
  command: "명령 실행 승인",
  writeStdin: "실행 중인 프로세스에 입력 보내기 승인",
  fileChange: "파일 변경 승인",
};

const DECISION_LABEL: Record<ApprovalDecision, string> = {
  accept: "허용",
  acceptForSession: "이 세션 동안 허용",
  decline: "거부",
};

function findFileChange(transcript: Transcript | null, turnId: string, itemId: string) {
  const item = transcript?.turns.find((turn) => turn.id === turnId)?.items.find((candidate) => candidate.id === itemId);
  return item?.type === "fileChange" ? item : null;
}

function ApprovalChanges({ turnId, itemId }: { turnId: string; itemId: string }) {
  const item = useDesktopStore((state) => findFileChange(state.transcript, turnId, itemId));
  const files = useMemo(() => item?.changes.map(diffFileFromChange) ?? [], [item]);
  if (files.length === 0) {
    return <div className="muted">변경 내용을 아직 받지 못했습니다.</div>;
  }
  return (
    <div className="approval-changes" data-testid="approval-changes">
      <DiffFileList files={files} defaultOpen />
    </div>
  );
}

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
            {APPROVAL_TITLE[approval.kind]}
            {approval.command && <code>{approval.command}</code>}
          </div>
          {approval.cwd && <div className="muted">{approval.cwd}</div>}
          {approval.reason && <div>{approval.reason}</div>}
          {approval.kind === "fileChange" && <ApprovalChanges turnId={approval.turnId} itemId={approval.itemId} />}
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
