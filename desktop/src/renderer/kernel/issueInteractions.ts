import type { IssueAutomationAnswer, IssueDecisionRequestState, IssueDetailAnswer, IssueEventReference, KernelCommand } from "./protocol";

export type ReadyIssueDetail = Extract<IssueDetailAnswer, { status: "ready" }>;
export type IssueAutonomyLevel = Extract<KernelCommand, { type: "set_issue_autonomy" }>["level"];
export type IssueAutonomyView = Extract<IssueAutomationAnswer, { status: "ready" }>["autonomy"];
export type IssueInterventionName = Extract<KernelCommand, { type:
  "hold_issue" | "release_issue" | "steer_issue" | "take_over_issue" |
  "retriage_issue" | "cancel_issue_work" | "track_issue" | "untrack_issue"
}>["type"];
export type IssueConversationTarget = ReadyIssueDetail["conversation"];

/** RPC 완료와 구분한 명령 결과다. 사용자 초안은 succeeded일 때만 비운다. */
export type IssueCommandOutcome = "succeeded" | "cancelled" | "rejected" | "failed" | "unknown" | "unavailable";

export const ISSUE_AUTONOMY_LABEL: Record<string, string> = {
  off: "끄기", observe: "관찰", propose: "제안", gated: "단계별 승인", auto: "자동",
};

export const ISSUE_ACTION_LABEL: Record<IssueInterventionName, string> = {
  hold_issue: "보류", release_issue: "보류 해제", steer_issue: "추가 지시",
  take_over_issue: "내가 맡기", retriage_issue: "다시 판정",
  cancel_issue_work: "작업 취소", track_issue: "추적 시작", untrack_issue: "추적 중지",
};

export const ISSUE_DECISION_STATE_LABEL: Record<string, string> = {
  open: "답변 대기", answered: "답변됨", expired: "기한 종료", withdrawn: "철회됨", superseded: "기준 변경됨",
};

export const ISSUE_DECISION_KIND_LABEL: Record<string, string> = {
  start_approval: "작업 시작", spec_confirmation: "사양 확인", plan_approval: "계획 승인",
  risk_exception: "위험 예외", issue_changed: "이슈 변경", failure_recovery: "실패 복구", agent_question: "작업자 질문", close_approval: "이슈 닫기 승인",
};

export function issueTextBytes(value: string): number {
  return new TextEncoder().encode(value).length;
}

export interface IssueDecisionDraft {
  readonly option: string;
  readonly note: string;
}

export interface IssueInterventionDraft {
  readonly message: string;
  readonly steering: boolean;
  readonly cancelling: boolean;
}

export const EMPTY_ISSUE_DECISION_DRAFT: IssueDecisionDraft = { option: "", note: "" };
export const EMPTY_ISSUE_INTERVENTION_DRAFT: IssueInterventionDraft = { message: "", steering: false, cancelling: false };

export function issueDraftIdentity(issue: IssueEventReference): string {
  return JSON.stringify([issue.provider, issue.server_id, issue.repository_id, issue.issue_id, issue.project_scope.workspace_dir]);
}

export function decisionDraftIdentity(request: IssueDecisionRequestState): string {
  const basis = request.basis;
  return JSON.stringify([
    issueDraftIdentity(request.subject.issue), request.id, request.revision, request.kind, request.gate, request.state,
    basis.policy_version, basis.gate_revision, basis.authorization_epoch ?? 0, basis.work_order_id ?? null,
    basis.field_hash?.version ?? null, basis.field_hash?.sha256 ?? null,
  ]);
}

/** 목록 전체 판이나 관찰 시각 대신 실제 작업 대상과 공개·허가 기준으로 사용자 지시 초안을 묶는다. */
export function interventionDraftIdentity(detail: ReadyIssueDetail): string {
  const observation = detail.item.current;
  const content = observation.status === "observed" ? observation.content : null;
  const control = detail.control;
  return JSON.stringify([
    issueDraftIdentity(detail.item.issue), detail.item.work?.work_order_id ?? null, [...(detail.item.work?.task_ids ?? [])].sort(),
    control?.authorization_epoch ?? null, control?.tracked ?? null, control?.held ?? null, control?.taken_over ?? null, control?.cancelled ?? null,
    observation.status, observation.status === "observed" ? observation.remote_state : null,
    content?.kind ?? null, content?.policy_version ?? null, content?.basis.field_hash.version ?? null, content?.basis.field_hash.sha256 ?? null,
    detail.item.triage?.policy_version ?? null, detail.conversation?.kind ?? null,
    detail.conversation?.kind === "thread" ? detail.conversation.thread_id : null,
  ]);
}
