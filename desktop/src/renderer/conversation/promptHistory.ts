// 보낸 프롬프트의 저장이다 (plan 26 I1). 이 컴퓨터의 localStorage 에만 둔다.
import { addPrompt, PROMPT_LIMIT } from "./composerKeys";

const PROMPTS_KEY = "akra.promptHistory";

export function loadPrompts(): string[] {
  try {
    const stored: unknown = JSON.parse(window.localStorage.getItem(PROMPTS_KEY) ?? "[]");
    return Array.isArray(stored) ? stored.filter((item): item is string => typeof item === "string").slice(-PROMPT_LIMIT) : [];
  } catch {
    return [];
  }
}

/** 보낸 글을 기억한다. 이 컴퓨터에만 저장한다. 저장하지 못하면 아무것도 하지 않는다. */
export function rememberPrompt(text: string): void {
  try {
    window.localStorage.setItem(PROMPTS_KEY, JSON.stringify(addPrompt(loadPrompts(), text)));
  } catch {
    // 저장하지 못했다.
  }
}
