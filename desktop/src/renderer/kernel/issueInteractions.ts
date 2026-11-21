import type { IssueAutomationAnswer, IssueDetailAnswer, KernelCommand } from "./protocol";

export type ReadyIssueDetail = Extract<IssueDetailAnswer, { status: "ready" }>;
export type IssueAutonomyLevel = Extract<KernelCommand, { type: "set_issue_autonomy" }>["level"];
export type IssueAutonomyView = Extract<IssueAutomationAnswer, { status: "ready" }>["autonomy"];
export type IssueInterventionName = Extract<KernelCommand, { type:
  "hold_issue" | "release_issue" | "steer_issue" | "take_over_issue" |
  "retriage_issue" | "cancel_issue_work" | "track_issue" | "untrack_issue"
}>["type"];
export type IssueConversationTarget = ReadyIssueDetail["conversation"];

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
  risk_exception: "위험 예외", issue_changed: "이슈 변경", failure_recovery: "실패 복구", agent_question: "작업자 질문",
};

export function issueTextBytes(value: string): number {
  return new TextEncoder().encode(value).length;
}
