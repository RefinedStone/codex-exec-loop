// 대화에서 고른 글을 입력창 초안에 붙이는 인용 형식이다 (#2842). 화면 모듈을 가져오지 않는다 (node 시험용).

/** 고른 글을 Markdown 인용으로 바꾼다. 줄마다 `> ` 를 붙인다. */
export function quoteText(text: string): string {
  return text.replace(/\s+$/, "").split(/\r?\n/).map((line) => (line ? `> ${line}` : ">")).join("\n");
}

/** 기존 초안 뒤에 인용을 붙인다. 초안을 지우지 않는다. 인용 뒤에 빈 줄을 두어 바로 이어 쓸 수 있게 한다. */
export function appendQuote(draft: string, text: string): string {
  const quote = `${quoteText(text)}\n\n`;
  const kept = draft.replace(/\s+$/, "");
  return kept ? `${kept}\n\n${quote}` : quote;
}
