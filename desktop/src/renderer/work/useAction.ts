// 단추 하나가 시작하는 동작의 상태이다: 쉬는 중 → 실행 중 → 쉬는 중 또는 실패. `busy` 와 `error` 를 따로 두지
// 않는다. 그래서 "실행 중인데 이전 오류가 보인다" 같은 조합이 없다.
import { useRef, useState } from "react";

export type ActionState = { phase: "idle" } | { phase: "running" } | { phase: "failed"; message: string };

export interface Action {
  busy: boolean;
  /** 마지막 실행이 실패한 이유이다. 다시 실행하면 지운다. */
  error: string | null;
  /** 동작을 실행한다. 실행 중에 다시 부르면 아무것도 하지 않는다. */
  run(action: () => Promise<void>): Promise<void>;
}

/** 동작을 한 번 실행하고 상태를 차례로 알린다. 실패는 던지지 않고 상태로 알린다. */
export async function runAction(action: () => Promise<void>, report: (state: ActionState) => void): Promise<void> {
  report({ phase: "running" });
  try {
    await action();
    report({ phase: "idle" });
  } catch (cause) {
    report({ phase: "failed", message: cause instanceof Error ? cause.message : String(cause) });
  }
}

export function useAction(): Action {
  const [state, setState] = useState<ActionState>({ phase: "idle" });
  // 그려진 `busy` 는 한 박자 늦다. 빠르게 두 번 눌러도 한 번만 실행하도록 지금 값을 따로 둔다.
  const running = useRef(false);
  const run = async (action: () => Promise<void>) => {
    if (running.current) return;
    running.current = true;
    await runAction(action, setState);
    running.current = false;
  };
  return { busy: state.phase === "running", error: state.phase === "failed" ? state.message : null, run };
}
