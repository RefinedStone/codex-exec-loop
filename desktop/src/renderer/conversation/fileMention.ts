// 입력창의 `@` 파일 언급이다 (#2837). 화면 모듈을 가져오지 않는다 (node 시험용).

export interface ActiveMention {
  /** `@` 의 자리이다. */
  start: number;
  /** `@` 뒤부터 커서까지의 글이다. 빈 글이면 모든 파일이다. */
  query: string;
}

/** 커서 바로 앞의 `@글` 이다. `@` 앞이 글의 처음이거나 공백일 때만 언급으로 본다 (메일 주소 같은 글은 아니다). */
export function activeMention(text: string, caret: number): ActiveMention | null {
  const before = text.slice(0, caret);
  const match = /(^|\s)@([^\s@]*)$/.exec(before);
  if (!match) return null;
  return { start: before.length - match[2]!.length - 1, query: match[2]! };
}

/** 두 언급이 같은 `@` 자리와 같은 글인지 본다. 커서가 움직여 언급이 바뀌었는지 가린다. */
export function sameMention(a: ActiveMention | null, b: ActiveMention | null): boolean {
  return a === b || (a !== null && b !== null && a.start === b.start && a.query === b.query);
}

/** `@글` 을 고른 경로로 바꾼다. 경로에 공백이 있으면 따옴표로 감싼다. 뒤에 공백을 하나 둔다. */
export function applyMention(text: string, mention: ActiveMention, caret: number, path: string): { text: string; caret: number } {
  const inserted = `${/\s/.test(path) ? JSON.stringify(path) : path} `;
  const next = text.slice(0, mention.start) + inserted + text.slice(caret);
  return { text: next, caret: mention.start + inserted.length };
}
