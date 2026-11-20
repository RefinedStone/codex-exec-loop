import { create } from "zustand";

/** 진행 중인 요청은 입력창 수명과 분리한다. 창을 다시 만들어도 같은 스레드를 중복 전송하지 않는다. */
export const useComposerRequests = create<{ pendingKeys: string[] }>(() => ({ pendingKeys: [] }));

export async function runComposerAction(key: string, action: () => Promise<void>): Promise<void> {
  if (useComposerRequests.getState().pendingKeys.includes(key)) return;
  useComposerRequests.setState((state) => ({ pendingKeys: [...state.pendingKeys, key] }));
  try {
    await action();
  } finally {
    useComposerRequests.setState((state) => ({ pendingKeys: state.pendingKeys.filter((candidate) => candidate !== key) }));
  }
}

/** 이전 입력창의 응답은 전송 뒤에 작성된 새 초안을 지울 수 없다. */
export function isSentComposerDraft(current: string | undefined, sent: string): boolean {
  return current === sent;
}
