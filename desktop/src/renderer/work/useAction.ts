// 단추 하나가 시작하는 동작의 상태이다: 쉬는 중 → 실행 중 → 쉬는 중 또는 실패. `busy` 와 `error` 를 따로 두지
// 않는다. 그래서 "실행 중인데 이전 오류가 보인다" 같은 조합이 없다.
import { useState } from "react";

export type ActionState = { phase: "idle" } | { phase: "running" } | { phase: "failed"; message: string };

export interface Action {
  busy: boolean;
  /** 마지막 실행이 실패한 이유이다. 다시 실행하면 지운다. */
  error: string | null;
  /** 동작을 실행한다. 실행 중에 다시 부르면 아무것도 하지 않는다. */
  run(action: () => Promise<void>): Promise<void>;
}

/**
 * 동작을 실행하고 상태를 차례로 알리는 함수를 만든다. 실패는 던지지 않고 상태로 알린다. 그려진 `busy` 는 한 박자
 * 늦으므로 실행 중인지를 여기에 따로 둔다: 빠르게 두 번 눌러도 한 번만 실행한다.
 */
export function actionRunner(report: (state: ActionState) => void): Action["run"] {
  let running = false;
  return async (action) => {
    if (running) return;
    running = true;
    report({ phase: "running" });
    try {
      await action();
      report({ phase: "idle" });
    } catch (cause) {
      report({ phase: "failed", message: cause instanceof Error ? cause.message : String(cause) });
    } finally {
      running = false;
    }
  };
}

export function useAction(): Action {
  const [state, setState] = useState<ActionState>({ phase: "idle" });
  const [run] = useState(() => actionRunner(setState));
  return { busy: state.phase === "running", error: state.phase === "failed" ? state.message : null, run };
}
