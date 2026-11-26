// 다시 연 스레드에서 "새 내용" 구분선을 둘 턴을 고른다 (#2843). 화면 모듈을 가져오지 않는다 (node 시험용).

/** 떠날 때의 마지막 턴 뒤의 첫 턴이다. 그 턴이 목록에 없거나 새 턴이 없으면 null 이다. */
export function newContentTurnId(turnIds: readonly string[], lastSeenTurnId: string | null | undefined): string | null {
  const index = lastSeenTurnId ? turnIds.indexOf(lastSeenTurnId) : -1;
  return index >= 0 ? (turnIds[index + 1] ?? null) : null;
}

/** 다시 열 때 보던 턴을 찾으려고 더 읽을 이전 턴 페이지 수의 상한이다. */
export const RESTORE_MAX_OLDER_PAGES = 5;

export type RestoreStep = { kind: "scroll"; index: number } | { kind: "loadOlder" } | { kind: "bottom" };

/**
 * 보던 턴으로 돌아갈 다음 걸음이다 (#2843). 처음에는 최근 페이지만 읽으므로 그 턴이 아직 없을 수 있다.
 * 그 턴이 있으면 거기로, 없고 이전 페이지가 남았으면(상한 안) 한 페이지 더 읽고, 아니면 맨 아래이다.
 */
export function restoreStep(turnIds: readonly string[], targetTurnId: string | null, hasOlder: boolean, olderPagesRead: number): RestoreStep {
  if (targetTurnId === null) return { kind: "bottom" };
  const index = turnIds.indexOf(targetTurnId);
  if (index >= 0) return { kind: "scroll", index };
  return hasOlder && olderPagesRead < RESTORE_MAX_OLDER_PAGES ? { kind: "loadOlder" } : { kind: "bottom" };
}

/** 떠날 때 남길 기억이다. 대화를 읽어 복원을 마치기 전에 떠났으면 이전 기억을 지우지 않는다. */
export function memoryOnLeave<T>(previous: T | undefined, restored: boolean, next: T): T | undefined {
  return restored ? next : previous;
}
