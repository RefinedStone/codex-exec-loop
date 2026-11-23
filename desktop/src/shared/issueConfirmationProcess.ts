import type { IssueConfirmationResponse } from "./issueConfirmation";

export type ConfirmationKind = "decision" | "mode" | "autonomy" | "binding_resume";
export interface ConfirmationDisplay {
  message: string;
  detail: string;
  confirm_label: string;
}

/** main이 만든 private utility channel 전용이다. renderer에는 이 채널을 전달하지 않는다. */
export type ConfirmationControl =
  | { type: "begin"; kind: ConfirmationKind; input: unknown; endpoint: string; executable: string }
  | { type: "answer"; approved: boolean }
  | { type: "submit"; allowed: boolean }
  | { type: "cancel" }
  | { type: "dispose" };

/** 등록, intent, proof, Gateway frame은 이 계약에 없다. */
export type ConfirmationEvent =
  | { type: "ready" }
  | { type: "prompt"; display: ConfirmationDisplay }
  | { type: "will-submit" }
  | { type: "result"; response: IssueConfirmationResponse }
  | { type: "failed" };
