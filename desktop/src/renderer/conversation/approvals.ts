// 승인 요청(서버 요청)을 화면용 값으로 바꾼다 (P3 §6.4). 명령 실행과 파일 변경만 다룬다.
// codex 0.159 의 안정 API 는 요청에 선택지 목록을 주지 않으므로, 응답 타입이 정한 값 중 세 가지만 보여 준다.
import type { JsonRpcId } from "../rpc/jsonRpcClient";

/** writeStdin 은 이미 실행 중인 프로세스(터미널)에 입력을 보내는 승인이다. 새 명령 실행과 구분해 보여 준다. */
export type ApprovalKind = "command" | "writeStdin" | "fileChange";
export type ApprovalDecision = "accept" | "acceptForSession" | "decline";

export const APPROVAL_DECISIONS: readonly ApprovalDecision[] = ["accept", "acceptForSession", "decline"];

export interface PendingApproval {
  requestId: JsonRpcId;
  kind: ApprovalKind;
  threadId: string;
  turnId: string;
  itemId: string;
  command: string | null;
  cwd: string | null;
  reason: string | null;
}

const APPROVAL_METHODS: Record<string, ApprovalKind> = {
  "item/commandExecution/requestApproval": "command",
  "item/fileChange/requestApproval": "fileChange",
};

function optionalString(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

/** 다루는 승인 요청이면 화면용 값을, 아니면 null 을 돌려준다. */
export function toPendingApproval(method: string, params: unknown, requestId: JsonRpcId): PendingApproval | null {
  const kind = APPROVAL_METHODS[method];
  const fields = params as Record<string, unknown> | null;
  if (!kind || !fields || typeof fields.threadId !== "string") {
    return null;
  }
  return {
    requestId,
    kind: kind === "command" && fields.kind === "writeStdin" ? "writeStdin" : kind,
    threadId: fields.threadId,
    turnId: String(fields.turnId ?? ""),
    itemId: String(fields.itemId ?? ""),
    command: optionalString(fields.command),
    cwd: optionalString(fields.cwd),
    reason: optionalString(fields.reason),
  };
}

export function approvalKey(requestId: JsonRpcId): string {
  return String(requestId);
}
