// 사용자에게 알릴 일을 저장소의 앞뒤 값에서 찾는다 (plan 26 A1): 턴이 끝났다, 승인 요청이 왔다.
// shortcut: 지금 있는 스레드 상태 값으로 계산한다. plan 23 의 주의 항목 목록이 생기면 그 목록을 읽게 바꾼다.
import type { DesktopState } from "./desktopStore";

export type AttentionKind = "finished" | "failed" | "approval";

export interface AttentionEvent {
  kind: AttentionKind;
  threadId: string;
}

type AttentionSource = Pick<DesktopState, "connection" | "activeThreadIds" | "pendingApprovals" | "folders">;

function isFailed(state: AttentionSource, threadId: string): boolean {
  return state.folders.some((folder) => folder.threads.some((thread) => thread.id === threadId && thread.failed));
}

/**
 * 접속이 끊겼다가 붙는 동안에는 목록을 새로 채우므로 알리지 않는다: 그때 사라진 "진행 중"은 지금 끝난 턴이 아닐 수
 * 있다.
 */
export function attentionEvents(previous: AttentionSource, next: AttentionSource): AttentionEvent[] {
  if (previous.connection?.phase !== "connected" || next.connection?.phase !== "connected") return [];
  const events: AttentionEvent[] = [];
  for (const approval of next.pendingApprovals) {
    if (!previous.pendingApprovals.some((known) => known.requestId === approval.requestId)) {
      events.push({ kind: "approval", threadId: approval.threadId });
    }
  }
  for (const threadId of previous.activeThreadIds) {
    if (!next.activeThreadIds.includes(threadId)) {
      events.push({ kind: isFailed(next, threadId) ? "failed" : "finished", threadId });
    }
  }
  return events;
}

export const ATTENTION_TEXT: Record<AttentionKind, string> = {
  finished: "턴이 끝났습니다.",
  failed: "턴이 실패했습니다.",
  approval: "승인을 기다립니다.",
};
