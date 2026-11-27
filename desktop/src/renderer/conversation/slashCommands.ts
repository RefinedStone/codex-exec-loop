import { command, PALETTE_ORDER, type CommandSpec } from "../../shared/commands";

/** 초안 전체의 /명령만 받는다. 경로, 인수가 있는 문장, 선택 영역은 일반 입력으로 남긴다. */
export function slashCommandQuery(text: string, start: number, end: number = start): string | null {
  if (start !== end || start !== text.length || !text.startsWith("/") || /[\s/]/.test(text.slice(1))) return null;
  return text.slice(1).toLowerCase();
}

/** 팔레트가 읽는 같은 명령 표와 순서를 쓴다. 별도 명령 이름이나 동작을 만들지 않는다. */
export function matchingSlashCommands(query: string): CommandSpec[] {
  const needle = query.toLowerCase();
  return PALETTE_ORDER.map(command).filter((spec) =>
    spec.id.startsWith(needle) || spec.title.toLowerCase().includes(needle)
      || spec.paletteTitle?.toLowerCase().includes(needle),
  );
}
