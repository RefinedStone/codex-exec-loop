// 대화 안 찾기이다 (plan 26 R1). 긴 대화는 보이는 턴만 그리므로, 화면이 아니라 대화 기록에서 찾는다.
import type { TurnView } from "./transcript";

function turnText(turn: TurnView): string {
  return turn.items
    .map((item) => {
      if (item.type === "agentMessage") return item.text;
      if (item.type === "userMessage") return item.content.map((part) => (part.type === "text" ? part.text : "")).join("\n");
      return "";
    })
    .join("\n");
}

/** 사용자 글이나 에이전트 답에 `query` 가 있는 턴의 자리이다. 대소문자를 가리지 않는다. 빈 글이면 없다. */
export function findTurns(turns: readonly TurnView[], query: string): number[] {
  const needle = query.trim().toLowerCase();
  if (needle.length === 0) return [];
  return turns.flatMap((turn, index) => (turnText(turn).toLowerCase().includes(needle) ? [index] : []));
}

/** `text` 안에서 `needle` 이 시작하는 자리이다. 둘 다 소문자로 준다. */
export function matchOffsets(text: string, needle: string): number[] {
  const offsets: number[] = [];
  if (needle.length === 0) return offsets;
  for (let from = text.indexOf(needle); from !== -1; from = text.indexOf(needle, from + needle.length)) {
    offsets.push(from);
  }
  return offsets;
}

/** 검색 결과를 원문 텍스트의 UTF-16 범위로 돌려준다. */
export function matchRanges(text: string, needle: string): Array<{ start: number; end: number }> {
  if (needle.length === 0) return [];
  const starts: number[] = [];
  const ends: number[] = [];
  let originalOffset = 0;
  for (const character of text) {
    const nextOffset = originalOffset + character.length;
    for (let unit = 0; unit < character.toLowerCase().length; unit += 1) {
      starts.push(originalOffset);
      ends.push(nextOffset);
    }
    originalOffset = nextOffset;
  }
  // 전체 문자열을 변환해야 그리스어 어말 시그마 같은 문맥 규칙도 유지된다.
  return matchOffsets(text.toLowerCase(), needle).map((offset) => ({
    start: starts[offset]!,
    end: ends[offset + needle.length - 1]!,
  }));
}
