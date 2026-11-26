import type { CommandResponse, IssueAutomationModeRecord, IssueAutonomyLevel, TaskAcceptanceCriterion } from "./kernel-protocol/generated";

/** 사람 확인 요청에는 명령 입력만 있다. 이 IPC에는 확인 증거와 서버 intent를 담지 않는다. */
export interface ConfirmIssueDecisionRequest {
  workspace: string;
  decisionId: string;
  criteria?: TaskAcceptanceCriterion[] | null;
  option: string;
  note?: string | null;
  key: string;
}

export interface ConfirmIssueAutomationModeRequest {
  workspace: string;
  mode: IssueAutomationModeRecord;
  key: string;
}

/** 자율 수준은 원격 수집 모드와 별도인 저장소 설정이다. */
export interface ConfirmIssueAutonomyRequest {
  workspace: string;
  level: IssueAutonomyLevel;
  commandId: string;
  key: string;
}

/** 재개는 현재 바인딩 판과 서버가 준비한 정책 기준에 대한 native 확인을 요구한다. */
export interface ConfirmIssueBindingResumeRequest {
  workspace: string;
  commandId: string;
  bindingId: string;
  expectedRevision: number;
  key: string;
}

export type IssueConfirmationResponse =
  | { cancelled: true }
  | { cancelled: false; response: CommandResponse }
  /** 제출을 시작한 뒤 결과를 받지 못했다. 명령을 자동 재전송하지 않고 서버 상태를 다시 읽는다. */
  | { cancelled: false; outcome: "unknown" };
