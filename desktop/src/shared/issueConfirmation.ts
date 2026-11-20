import type { CommandResponse, IssueAutomationModeRecord } from "./kernel-protocol/generated";

/** 사람 확인 요청에는 명령 입력만 있다. 확인 증거와 서버 intent는 main 밖으로 나오지 않는다. */
export interface ConfirmIssueDecisionRequest {
  workspace: string;
  decisionId: string;
  option: string;
  note?: string | null;
  key: string;
}

export interface ConfirmIssueAutomationModeRequest {
  workspace: string;
  mode: IssueAutomationModeRecord;
  key: string;
}

export type IssueConfirmationResponse =
  | { cancelled: true }
  | { cancelled: false; response: CommandResponse }
  /** 제출을 시작한 뒤 결과를 받지 못했다. 명령을 자동 재전송하지 않고 서버 상태를 다시 읽는다. */
  | { cancelled: false; outcome: "unknown" };
