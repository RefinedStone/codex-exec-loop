import type { IssueArbiterWaitReason, IssueDecisionBasisRecord, IssueDecisionFactRecord, IssueDetailAnswer, KernelEvent } from "./protocol";

export type IssueArbiterFinishedRecord = Extract<Extract<KernelEvent, { type: "recorded" }>["event"], { kind: "issue_decision_arbiter_finished" }>;

export const ISSUE_ARBITER_WAIT_TEXT: Record<IssueArbiterWaitReason, string> = {
  low_confidence: "중재자의 판단 확신이 기준에 미치지 못했습니다.",
  blockers_present: "중재자가 진행을 막는 조건을 확인했습니다.",
  invalid_response: "중재자의 응답을 유효한 결정으로 확인하지 못했습니다.",
  outcome_unknown: "중재자의 검토 결과를 확인하지 못했습니다.",
  model_unavailable: "중재자 검토를 진행할 수 없습니다.",
  budget_exhausted: "중재자 검토 예산 한도에 도달했습니다.",
  evidence_unavailable: "검토에 필요한 근거를 확인하지 못했습니다.",
  evidence_changed: "검토 중 저장소 근거가 바뀌었습니다.",
  context_unavailable: "현재 결정 기준을 확인하지 못했습니다.",
};

export function isIssueArbiterWaitReason(value: unknown): value is IssueArbiterWaitReason {
  return typeof value === "string" && Object.hasOwn(ISSUE_ARBITER_WAIT_TEXT, value);
}

export function issueDecisionAnswerOrigin(by?: IssueDecisionFactRecord["answer_by"]): string | null {
  switch (by) {
    case "human": return "사람이 답변했습니다.";
    case "agent": return "에이전트가 답변했습니다.";
    case "default": return "기한에 따라 기본 선택을 적용했습니다.";
    case "arbiter": return "중재자가 답변했습니다.";
    default: return null;
  }
}

function sameBasis(left: IssueDecisionBasisRecord, right: IssueDecisionBasisRecord): boolean {
  return left.policy_version === right.policy_version && left.gate_revision === right.gate_revision
    && (left.authorization_epoch ?? 0) === (right.authorization_epoch ?? 0)
    && (left.work_order_id ?? null) === (right.work_order_id ?? null)
    && (left.field_hash?.version ?? null) === (right.field_hash?.version ?? null)
    && (left.field_hash?.sha256 ?? null) === (right.field_hash?.sha256 ?? null)
    && (left.delivery?.base_commit_sha ?? null) === (right.delivery?.base_commit_sha ?? null)
    && (left.delivery?.source_commit_sha ?? null) === (right.delivery?.source_commit_sha ?? null);
}

/** 과거 이벤트의 Open fact는 알림 허가가 아니다. 현재 조회의 결정/이력과 일치할 때만 알린다. */
export function currentIssueArbiterNotice(event: IssueArbiterFinishedRecord, detail: IssueDetailAnswer): "applied" | "waiting" | null {
  if (detail.status !== "ready" || detail.revision < event.revision) return null;
  const fact = event.decision;
  if (event.outcome.kind === "waiting" && fact.state === "open") {
    const current = detail.open_decisions?.find((request) => request.id === fact.decision_id);
    return current?.state === "open" && current.on_timeout === "arbiter"
      && current.arbiter_wait?.reason === event.outcome.reason
      && current.kind === fact.kind && current.gate === fact.gate && sameBasis(current.basis, fact.basis)
      ? "waiting" : null;
  }
  if (event.outcome.kind === "applied" && fact.state === "answered" && fact.answer_by === "arbiter") {
    const current = detail.decision_history?.find((entry) => entry.decision.decision_id === fact.decision_id)?.decision;
    return current?.state === "answered" && current.answer_by === "arbiter" && current.option === fact.option
      && current.kind === fact.kind && current.gate === fact.gate && sameBasis(current.basis, fact.basis)
      ? "applied" : null;
  }
  return null;
}
