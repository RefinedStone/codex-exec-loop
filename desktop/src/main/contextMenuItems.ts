// 오른쪽 클릭 메뉴의 항목을 정한다 (plan 26 C1). Electron 의 `context-menu` 이벤트 값만 읽는다.
// 메뉴를 만드는 일은 main 이 한다. 여기는 무엇을 보일지만 정하므로 Electron 없이 시험한다.

export type EditRole = "cut" | "copy" | "paste" | "pasteAndMatchStyle" | "selectAll";

export type ContextItem =
  | { kind: "suggestion"; word: string }
  | { kind: "learn"; word: string }
  | { kind: "copyLink"; url: string }
  | { kind: "edit"; role: EditRole; label: string; enabled: boolean }
  | { kind: "separator" };

export interface ContextTarget {
  isEditable: boolean;
  selectionText: string;
  linkURL: string;
  misspelledWord: string;
  dictionarySuggestions: readonly string[];
  editFlags: { canCut: boolean; canCopy: boolean; canPaste: boolean; canSelectAll: boolean };
}

const MAX_SUGGESTIONS = 5;

function withoutEdgeSeparators(items: ContextItem[]): ContextItem[] {
  const kept = items.filter((item, index) => item.kind !== "separator" || (index > 0 && items[index - 1]?.kind !== "separator"));
  while (kept.at(-1)?.kind === "separator") kept.pop();
  return kept;
}

/** 보일 항목이 없으면 빈 목록이다. 그때는 메뉴를 띄우지 않는다. */
export function contextMenuItems(target: ContextTarget): ContextItem[] {
  const items: ContextItem[] = [];
  if (target.isEditable && target.misspelledWord) {
    for (const word of target.dictionarySuggestions.slice(0, MAX_SUGGESTIONS)) {
      items.push({ kind: "suggestion", word });
    }
    items.push({ kind: "learn", word: target.misspelledWord }, { kind: "separator" });
  }
  if (target.linkURL) {
    items.push({ kind: "copyLink", url: target.linkURL }, { kind: "separator" });
  }
  const flags = target.editFlags;
  if (target.isEditable) {
    items.push(
      { kind: "edit", role: "cut", label: "잘라내기", enabled: flags.canCut },
      { kind: "edit", role: "copy", label: "복사", enabled: flags.canCopy },
      { kind: "edit", role: "paste", label: "붙여넣기", enabled: flags.canPaste },
      { kind: "edit", role: "pasteAndMatchStyle", label: "서식 없이 붙여넣기", enabled: flags.canPaste },
      { kind: "separator" },
      { kind: "edit", role: "selectAll", label: "전체 선택", enabled: flags.canSelectAll },
    );
  } else if (target.selectionText.trim().length > 0) {
    items.push({ kind: "edit", role: "copy", label: "복사", enabled: flags.canCopy });
  }
  return withoutEdgeSeparators(items);
}
