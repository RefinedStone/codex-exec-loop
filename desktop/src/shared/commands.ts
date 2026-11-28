// 앱 명령 표이다 (#2840). 팔레트, 단축키 창, renderer 의 단축키 처리, main 의 앱 메뉴가 이 표 하나를 읽는다.
// 명령 하나를 더하면 세 곳에 함께 나온다. 동작(무엇을 하는지)은 각 프로세스가 id 로 정한다.

export type CommandId =
  | "palette"
  | "new-thread"
  | "sidebar"
  | "right-panel"
  | "terminal"
  | "find"
  | "back"
  | "forward"
  | "fleet"
  | "next-attention"
  | "review"
  | "graph"
  | "kernel"
  | "browser"
  | "work"
  | "docs"
  | "skills"
  | "theme"
  | "settings"
  | "new-window"
  | "zoom-in"
  | "zoom-out"
  | "zoom-reset";

export interface CommandSpec {
  id: CommandId;
  /** 단축키 창과 메뉴의 이름이다. */
  title: string;
  /** 팔레트의 이름이다. 없으면 `title` 이다. */
  paletteTitle?: string;
  /** 팔레트에 보일 때의 아이콘 이름이다. 없으면 팔레트에 없다. */
  paletteIcon?: string;
  /**
   * renderer 가 처리하는 `Cmd/Ctrl` + 글쇠이다. 값은 `KeyboardEvent.code` 이다 (한글 입력 상태에서도 같은 자리).
   * Shift·Alt 조합은 받지 않는다.
   */
  keys?: readonly string[];
  /** main 의 앱 메뉴가 처리하는 단축키이다 (Electron accelerator). */
  accelerator?: string;
  /** 단축키 창에 보일 글쇠 글이다. `Cmd/Ctrl` 표시는 앞에 붙인다. */
  keyLabel?: string;
}

export const COMMANDS: readonly CommandSpec[] = [
  { id: "palette", title: "검색과 명령", keys: ["KeyK"], keyLabel: "K" },
  { id: "new-thread", title: "새 스레드", paletteIcon: "pencil", keys: ["KeyN"], keyLabel: "N" },
  { id: "sidebar", title: "사이드바 열기 / 닫기", keys: ["KeyB"], keyLabel: "B" },
  { id: "right-panel", title: "오른쪽 패널 열기 / 닫기", keys: ["Backslash"], keyLabel: "\\" },
  { id: "terminal", title: "터미널 열기 / 닫기", paletteIcon: "terminal", keys: ["KeyJ", "Backquote"], keyLabel: "J" },
  { id: "settings", title: "설정", paletteTitle: "설정 (글자 크기, 알림, 잠자기 방지)", paletteIcon: "sliders", accelerator: "CmdOrCtrl+,", keyLabel: "," },
  { id: "back", title: "뒤로 (마우스 뒤로 단추)", keys: ["BracketLeft"], keyLabel: "[" },
  { id: "forward", title: "앞으로 (마우스 앞으로 단추)", keys: ["BracketRight"], keyLabel: "]" },
  { id: "find", title: "대화 안 찾기 (⏎ 다음, ⇧⏎ 이전)", keys: ["KeyF"], keyLabel: "F" },
  { id: "zoom-in", title: "포커스한 영역 확대", accelerator: "CmdOrCtrl+Plus", keyLabel: "+" },
  { id: "zoom-out", title: "포커스한 영역 축소", accelerator: "CmdOrCtrl+-", keyLabel: "−" },
  { id: "zoom-reset", title: "포커스한 영역 실제 크기", accelerator: "CmdOrCtrl+0", keyLabel: "0" },
  { id: "new-window", title: "새 창", accelerator: "CmdOrCtrl+Shift+N", keyLabel: "⇧N" },
  { id: "next-attention", title: "다음 응답 필요 스레드", paletteIcon: "message", keys: ["KeyY"], keyLabel: "Y" },
  { id: "fleet", title: "현황판 열기", paletteIcon: "grid" },
  { id: "review", title: "변경 검토 패널", paletteIcon: "diff", keyLabel: "\\" },
  { id: "graph", title: "커밋 그래프 패널", paletteIcon: "branch" },
  { id: "kernel", title: "Akra 커널 패널", paletteIcon: "bolt" },
  { id: "browser", title: "브라우저 패널", paletteIcon: "globe" },
  { id: "work", title: "업무 패널 (GitHub, Jira, Notion)", paletteIcon: "briefcase" },
  { id: "docs", title: "문서 패널", paletteIcon: "book" },
  { id: "skills", title: "설치된 스킬 패널 (출처, 업데이트)", paletteIcon: "stack" },
  { id: "theme", title: "밝은 화면 / 어두운 화면", paletteIcon: "sun" },
];

/** 팔레트에 보이는 명령의 순서이다. 이전 팔레트와 같은 순서이다. */
export const PALETTE_ORDER: readonly CommandId[] = [
  "new-thread", "fleet", "next-attention", "review", "graph", "kernel", "browser", "work", "docs", "skills", "terminal", "theme", "settings",
];

export function command(id: CommandId): CommandSpec {
  const found = COMMANDS.find((spec) => spec.id === id);
  if (!found) throw new Error(`unknown command ${id}`);
  return found;
}

/** `Cmd/Ctrl` + 글쇠의 명령이다. 없으면 null 이다. */
export function commandForKey(code: string): CommandId | null {
  return COMMANDS.find((spec) => spec.keys?.includes(code))?.id ?? null;
}

/**
 * 단축키를 비교할 수 있는 글로 바꾼다. renderer 의 `KeyboardEvent.code` 와 메뉴의 accelerator 를 같은 모양으로 만든다.
 * 충돌 시험이 쓴다.
 */
export function shortcutIdentity(spec: CommandSpec): string[] {
  const fromCode = (code: string) =>
    code.startsWith("Key") ? code.slice(3) : ({ Backslash: "\\", Backquote: "`", BracketLeft: "[", BracketRight: "]", Comma: "," } as Record<string, string>)[code] ?? code;
  const keys = (spec.keys ?? []).map((code) => `Mod+${fromCode(code)}`);
  const menu = spec.accelerator ? [spec.accelerator.replace("CmdOrCtrl", "Mod")] : [];
  return [...keys, ...menu];
}
