import type { KernelCommand, PlanningState, TaskEvidenceTaskState } from "./protocol";
import type { PlanningEvidenceFreshness } from "./kernelConnection";

export type TaskVerificationTarget = Pick<Extract<KernelCommand, { type: "request_task_verification" }>,
  "task_id" | "criterion_id" | "scope_id" | "expected_acceptance_revision">;

export interface TaskVerificationRequestView {
  /** `replayed`: the kernel confirmed the original request but a replay carries no attempt, so the next click is a new request. */
  status: "sending" | "accepted" | "replayed" | "unknown" | "rejected";
  requestId: string;
  attemptId: string | null;
  message: string;
}
export type TaskVerificationRequests = Readonly<Record<string, TaskVerificationRequestView>>;

export function verificationTarget(task: TaskEvidenceTaskState, criterionId: string): TaskVerificationTarget {
  return {
    task_id: task.scope.task_id, criterion_id: criterionId, scope_id: task.scope.scope_id,
    expected_acceptance_revision: task.scope.acceptance_revision,
  };
}

export function verificationTargetKey(target: TaskVerificationTarget): string {
  return JSON.stringify([target.task_id, target.criterion_id, target.scope_id, target.expected_acceptance_revision]);
}

/** UI eligibility never replaces the kernel's current lease/scope checks. */
export function verificationBlockReason(
  snapshot: PlanningState["task_evidence"], freshness: PlanningEvidenceFreshness,
  target: TaskVerificationTarget, request?: TaskVerificationRequestView,
): string | null {
  if (freshness !== "current" || !snapshot || snapshot.unavailable) return "현재 기준을 다시 조회한 뒤 요청할 수 있습니다.";
  const task = snapshot.tasks.find((entry) => entry.scope.task_id === target.task_id);
  if (!task || task.unavailable || task.scope.scope_id !== target.scope_id
    || task.scope.acceptance_revision !== target.expected_acceptance_revision) return "작업의 완료 기준이 바뀌었습니다. 다시 조회해 주세요.";
  const entry = task.criteria.find((item) => item.criterion.id === target.criterion_id);
  if (!entry || entry.criterion.channel !== "command" || entry.criterion.verification_recipe !== "git_diff_check_v1") {
    return "이 기준에는 실행할 수 있는 호스트 검증이 설정되지 않았습니다.";
  }
  if (task.source?.kind !== "running_lease") return "실행 중인 작업에서 호스트 검증을 요청할 수 있습니다.";
  if (request?.status === "sending") return "요청을 보내는 중입니다.";
  const host = entry.host_verification;
  if (host?.phase === "running" || (host?.phase === "pending" && host.attempt)) return "호스트 검증이 예약되었거나 실행 중입니다.";
  if (request?.status === "accepted") {
    const observed = request.attemptId !== null && host?.phase === "finished"
      && host.attempt?.attempt_id === request.attemptId;
    if (!observed) return "접수한 요청의 결과를 확인하고 있습니다.";
  }
  return null;
}
