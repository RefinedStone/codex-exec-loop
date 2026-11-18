// 입력창의 글쇠 판정과 이전 프롬프트 불러오기이다 (plan 26 I1, I9).

export const PROMPT_LIMIT = 50;

interface SendKeyEvent {
  key: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  keyCode: number;
  nativeEvent: { isComposing: boolean };
}

/**
 * Enter 가 보내기인지 정한다. 한글 같은 IME 조합 중의 Enter 는 조합을 끝내는 글쇠이다: 보내지 않는다. 조합 중에는
 * `isComposing` 이 true 이거나 `keyCode` 가 229 이다.
 */
export function isSendKey(event: SendKeyEvent): boolean {
  return event.key === "Enter" && !event.shiftKey && !event.ctrlKey && !event.metaKey && !event.nativeEvent.isComposing && event.keyCode !== 229;
}

/** 보낸 글을 목록 끝에 더한다. 바로 앞과 같은 글은 더하지 않는다. 오래된 것부터 버린다. */
export function addPrompt(prompts: readonly string[], text: string): string[] {
  if (text.trim().length === 0 || prompts.at(-1) === text) return [...prompts];
  return [...prompts, text].slice(-PROMPT_LIMIT);
}

/**
 * ↑ / ↓ 로 옮긴 자리이다. `cursor` 는 목록의 자리이고, null 이면 불러온 글이 없는 상태이다. ↑ 는 더 오래된 글로
 * 가고 가장 오래된 글에서 멈춘다. ↓ 는 더 새 글로 가고 가장 새 글 다음은 null 이다 (빈 입력창).
 */
export function recallCursor(count: number, cursor: number | null, direction: "older" | "newer"): number | null {
  if (count === 0) return null;
  if (direction === "older") return cursor === null ? count - 1 : Math.max(0, cursor - 1);
  if (cursor === null) return null;
  return cursor + 1 < count ? cursor + 1 : null;
}
