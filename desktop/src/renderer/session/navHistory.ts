// 이동 기록이다 (plan 26 N1, U-2). 보기(새 스레드, 스레드, 현황판, 원격 호스트)와 열린 스레드만 적는다.
// 오른쪽 패널을 열고 닫는 것은 적지 않는다. 기록은 창마다 따로 있다.

export type NavEntry =
  | { view: "home" }
  | { view: "fleet" }
  | { view: "thread"; threadId: string }
  | { view: "remote"; host: string; threadId: string };

export interface NavHistory {
  entries: readonly NavEntry[];
  /** 지금 보고 있는 항목의 자리이다. */
  index: number;
}

export const INITIAL_NAV: NavHistory = { entries: [{ view: "home" }], index: 0 };

const NAV_LIMIT = 100;

export function sameNavEntry(left: NavEntry, right: NavEntry): boolean {
  if (left.view === "thread" && right.view === "thread") return left.threadId === right.threadId;
  if (left.view === "remote" && right.view === "remote") return left.host === right.host && left.threadId === right.threadId;
  return left.view === right.view;
}

/** 새로 간 곳을 적는다. 뒤로 간 상태였으면 그 앞의 기록은 버린다. 같은 곳을 다시 열면 적지 않는다. */
export function pushNav(history: NavHistory, entry: NavEntry): NavHistory {
  const current = history.entries[history.index];
  if (current && sameNavEntry(current, entry)) return history;
  const entries = [...history.entries.slice(0, history.index + 1), entry].slice(-NAV_LIMIT);
  return { entries, index: entries.length - 1 };
}

/**
 * 뒤(`-1`)나 앞(`1`)으로 한 번 간 기록이다. 갈 곳이 없으면 null 이다. `exists` 가 false 인 항목(지워진 스레드,
 * 꺼진 원격 호스트)은 건너뛴다. 건너뛴 뒤에 지금과 같은 곳이 나오면 그것도 건너뛴다.
 */
export function stepNav(history: NavHistory, delta: -1 | 1, exists: (entry: NavEntry) => boolean): NavHistory | null {
  const current = history.entries[history.index];
  for (let index = history.index + delta; index >= 0 && index < history.entries.length; index += delta) {
    const entry = history.entries[index];
    if (entry && exists(entry) && !(current && sameNavEntry(current, entry))) {
      return { entries: history.entries, index };
    }
  }
  return null;
}
