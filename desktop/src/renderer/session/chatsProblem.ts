// ChatGPT 대화 조회(plan 25)의 실패와 제한을 화면 글자로 옮긴다. 목록(사이드바)과 대화(옆 칸)가 함께 쓴다.
import { JsonRpcError } from "../rpc/jsonRpcClient";

/** 기록 조회 제한의 JSON-RPC 오류 코드이다. Gateway 의 `akra/chats` 와 같아야 한다. */
export const CHATS_RATE_LIMITED = -32010;
const METHOD_NOT_FOUND = -32601;

/** Gateway 가 답에 붙이는 제한 상태이다. `limited` 이면 값은 제한 전에 마지막으로 읽은 것이다. */
export interface ChatsLimitState {
  limited?: boolean;
  retryAfterSeconds?: number | null;
}

/** 조회 제한의 안내 글자이다. 기다릴 시간을 알면 함께 적는다. */
export function limitNotice(retryAfterSeconds: unknown): string {
  const seconds = typeof retryAfterSeconds === "number" ? retryAfterSeconds : Number.NaN;
  const wait = Number.isFinite(seconds) && seconds > 0 ? ` ${Math.ceil(seconds)}초 뒤 다시 시도하세요.` : "";
  return `ChatGPT 가 기록 조회를 잠시 제한했습니다.${wait} 켜 둔 ChatGPT 앱이 같은 제한을 함께 씁니다.`;
}

/** 제한 중에 받은 답이면 "마지막으로 읽은 내용"이라는 안내를, 방금 읽은 답이면 null 을 돌린다. */
export function staleNotice(state: ChatsLimitState): string | null {
  return state.limited === true ? `마지막으로 읽은 내용입니다. ${limitNotice(state.retryAfterSeconds)}` : null;
}

export function isRateLimited(error: unknown): boolean {
  return error instanceof JsonRpcError && error.code === CHATS_RATE_LIMITED;
}

/** 조회 제한 오류이면 재시도 안내 글자를, 아니면 null 을 돌린다. */
export function rateLimitedMessage(error: unknown): string | null {
  if (!isRateLimited(error)) return null;
  const data = (error as JsonRpcError).data;
  return limitNotice(typeof data === "object" && data !== null && "retryAfterSeconds" in data ? (data as { retryAfterSeconds: unknown }).retryAfterSeconds : null);
}

/**
 * 목록 조회의 실패를 사이드바에 보일 글자로 옮긴다. chats 가 없는 Gateway(모르는 method)는 문제가 아니다: null 이고,
 * 구역을 숨긴다. 그 밖의 실패는 글자를 보이고 사용자가 다시 읽을 수 있게 한다.
 */
export function chatsListProblem(error: unknown): string | null {
  if (error instanceof JsonRpcError && error.code === METHOD_NOT_FOUND) return null;
  return rateLimitedMessage(error) ?? (error instanceof Error ? error.message : String(error));
}
