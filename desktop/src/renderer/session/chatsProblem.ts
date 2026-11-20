// ChatGPT 대화 조회(plan 25)의 실패를 화면 글자로 옮긴다. 목록(사이드바)과 대화(옆 칸)가 함께 쓴다.
import { JsonRpcError } from "../rpc/jsonRpcClient";

/** 기록 조회 제한의 JSON-RPC 오류 코드이다. Gateway 의 `akra/chats` 와 같아야 한다. */
export const CHATS_RATE_LIMITED = -32010;
const METHOD_NOT_FOUND = -32601;

/** 조회 제한 오류이면 재시도 안내 글자를, 아니면 null 을 돌린다. */
export function rateLimitedMessage(error: unknown): string | null {
  if (!(error instanceof JsonRpcError) || error.code !== CHATS_RATE_LIMITED) return null;
  const seconds =
    typeof error.data === "object" && error.data !== null && "retryAfterSeconds" in error.data
      ? Number((error.data as { retryAfterSeconds: unknown }).retryAfterSeconds)
      : Number.NaN;
  const wait = Number.isFinite(seconds) && seconds > 0 ? ` ${Math.ceil(seconds)}초 뒤 다시 시도하세요.` : "";
  return `ChatGPT 가 기록 조회를 잠시 제한했습니다.${wait} 켜 둔 ChatGPT 앱이 같은 제한을 함께 씁니다.`;
}

/**
 * 목록 조회의 실패를 사이드바에 보일 글자로 옮긴다. chats 가 없는 Gateway(모르는 method)는 문제가 아니다: null 이고,
 * 구역을 숨긴다. 그 밖의 실패는 글자를 보이고 사용자가 다시 읽을 수 있게 한다.
 */
export function chatsListProblem(error: unknown): string | null {
  if (error instanceof JsonRpcError && error.code === METHOD_NOT_FOUND) return null;
  return rateLimitedMessage(error) ?? (error instanceof Error ? error.message : String(error));
}
