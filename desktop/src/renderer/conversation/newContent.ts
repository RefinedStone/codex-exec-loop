// 다시 연 스레드에서 "새 내용" 구분선을 둘 턴을 고른다 (#2843). 화면 모듈을 가져오지 않는다 (node 시험용).

/** 떠날 때의 마지막 턴 뒤의 첫 턴이다. 그 턴이 목록에 없거나 새 턴이 없으면 null 이다. */
export function newContentTurnId(turnIds: readonly string[], lastSeenTurnId: string | null | undefined): string | null {
  const index = lastSeenTurnId ? turnIds.indexOf(lastSeenTurnId) : -1;
  return index >= 0 ? (turnIds[index + 1] ?? null) : null;
}
