import type { PlanningEvidenceFreshness } from "./kernelConnection";
import type { PlanningState } from "./protocol";
import type { TaskVerificationTarget } from "./taskVerificationActions";

export type TaskCriterionWaiverTarget = TaskVerificationTarget;
export interface TaskCriterionWaiverRequestView {
  status: "confirming" | "accepted" | "unknown" | "rejected";
  message: string;
}
export type TaskCriterionWaiverOutcome = "cancelled" | "accepted" | "unknown" | "rejected";
export interface TaskCriterionWaiverActions {
  waiverRequests?: Readonly<Record<string, TaskCriterionWaiverRequestView>>;
  onWaiveCriterion?(target: TaskCriterionWaiverTarget, reason: string): Promise<TaskCriterionWaiverOutcome>;
}

export function waiverReasonValid(reason: string): boolean {
  return reason.trim().length > 0 && new TextEncoder().encode(reason).byteLength <= 4096;
}

export function waiverBlockReason(snapshot: PlanningState["task_evidence"], freshness: PlanningEvidenceFreshness,
  target: TaskCriterionWaiverTarget): string | null {
  if (freshness !== "current" || !snapshot || snapshot.unavailable) return "현재 완료 기준을 다시 조회한 뒤 면제할 수 있습니다.";
  const task = snapshot.tasks.find((entry) => entry.scope.task_id === target.task_id);
  if (!task || task.unavailable || task.scope.scope_id !== target.scope_id
    || task.scope.acceptance_revision !== target.expected_acceptance_revision) return "완료 기준의 범위가 바뀌었습니다. 다시 조회해 주세요.";
  const entry = task.criteria.find((item) => item.criterion.id === target.criterion_id);
  if (!entry) return "이 완료 기준을 찾을 수 없습니다.";
  if (entry.state === "waived") return "현재 범위에 사람 면제가 기록되어 있습니다.";
  return null;
}
