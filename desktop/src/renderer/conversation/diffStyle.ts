// diff 를 한 줄로 볼지 좌우로 볼지의 선택이다 (P-12). 모든 diff 가 같은 값을 쓴다. 창을 다시 열어도 남는다.
import { useSyncExternalStore } from "react";

export type DiffStyle = "split" | "unified";

const KEY = "akra.diffStyle";
const listeners = new Set<() => void>();

function stored(): DiffStyle {
  try {
    return window.localStorage.getItem(KEY) === "unified" ? "unified" : "split";
  } catch {
    return "split";
  }
}

let current: DiffStyle = stored();

export function setDiffStyle(style: DiffStyle): void {
  current = style;
  try {
    window.localStorage.setItem(KEY, style);
  } catch {
    // 저장하지 못하면 이 창에서만 바뀐다.
  }
  listeners.forEach((listener) => listener());
}

export function useDiffStyle(): DiffStyle {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => current,
  );
}
