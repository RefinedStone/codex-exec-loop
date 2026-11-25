import { useState } from "react";
import type { PlanningEvidenceFreshness } from "./kernelConnection";
import type { PlanningCriterionEvidenceState, PlanningState, TaskEvidenceTaskState } from "./protocol";
import { verificationTarget, verificationTargetKey } from "./taskVerificationActions";
import { waiverBlockReason, waiverReasonValid, type TaskCriterionWaiverActions } from "./taskCriterionWaiver";

export function TaskCriterionWaiverView({ task, entry, snapshot, freshness, waiverRequests, onWaiveCriterion }: {
  task: TaskEvidenceTaskState;
  entry: PlanningCriterionEvidenceState;
  snapshot: NonNullable<PlanningState["task_evidence"]>;
  freshness: PlanningEvidenceFreshness;
} & TaskCriterionWaiverActions) {
  const [reason, setReason] = useState("");
  const [notice, setNotice] = useState<string | null>(null);
  if (!onWaiveCriterion) return null;
  const target = verificationTarget(task, entry.criterion.id);
  const request = waiverRequests?.[verificationTargetKey(target)];
  const blocked = waiverBlockReason(snapshot, freshness, target);
  const confirming = request?.status === "confirming";
  const unknown = request?.status === "unknown";
  const disabled = blocked !== null || confirming || request?.status === "accepted";
  return (
    <details className="task-evidence-record" data-testid="task-criterion-waiver">
      <summary>사람 면제</summary>
      <p className="muted">이 범위의 완료 기준을 면제합니다. 작업자 보고와 호스트 검증 결과는 그대로 남습니다. 시스템 확인 창에서 대상을 확인한 뒤 승인해 주세요.</p>
      <label>면제 사유
        <textarea data-testid="task-waiver-reason" value={reason} disabled={disabled || unknown} rows={3}
          onChange={(event) => { setReason(event.target.value); setNotice(null); }} />
      </label>
      <p className="muted">사유는 최대 4,096바이트입니다. 증거 조회에는 사유의 SHA-256만 표시됩니다.</p>
      <button type="button" data-testid="task-waiver-confirm" disabled={disabled || (!unknown && !waiverReasonValid(reason))}
        title={blocked ?? undefined} onClick={() => {
          setNotice(null);
          void onWaiveCriterion(target, reason).then((outcome) => {
            if (outcome === "cancelled") setNotice("면제를 취소했습니다. 완료 기준은 바뀌지 않았습니다.");
            if (outcome === "accepted") setReason("");
          });
        }}>
        {confirming ? "시스템 확인 중…" : unknown ? "같은 면제 요청 확인" : "시스템 확인 후 면제"}
      </button>
      {blocked && <p className="muted">{blocked}</p>}
      {(request || notice) && <p role="status" data-testid="task-waiver-notice"
        className={unknown || request?.status === "rejected" ? "warning" : "muted"}>{notice ?? request?.message}</p>}
    </details>
  );
}
