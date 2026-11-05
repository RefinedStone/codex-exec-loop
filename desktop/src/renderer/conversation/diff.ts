// 파일 변경을 화면용 줄 목록으로 바꾼다 (R1). 모두 순수 함수이다.
// `fileChange` 아이템의 diff 형식 (codex 0.159, 실측 2026-11-06): add/delete 는 파일 내용 그대로,
// update 는 `@@` hunk 부터 시작하는 unified diff (`---`/`+++` 머리 없음).
// `turn/diff/updated` 의 diff 는 파일마다 `diff --git` 머리가 붙은 git 형식이다.
import type { FileUpdateChange } from "../../shared/protocol/generated/v2/FileUpdateChange";
import type { ThreadItem } from "../../shared/protocol/generated/v2/ThreadItem";

export type DiffLineKind = "add" | "del" | "context" | "hunk" | "meta";
export type DiffFileKind = "add" | "delete" | "update";

export interface DiffLine {
  kind: DiffLineKind;
  text: string;
}

export interface DiffFile {
  path: string;
  kind: DiffFileKind;
  /** 이름이 바뀐 경우 새 경로이다. */
  movePath: string | null;
  lines: DiffLine[];
  added: number;
  removed: number;
}

function splitLines(text: string): string[] {
  const lines = text.split(/\r?\n/);
  if (lines.at(-1) === "") {
    lines.pop();
  }
  return lines;
}

function hunkLine(line: string): DiffLine {
  if (line.startsWith("@@")) return { kind: "hunk", text: line };
  if (line.startsWith("+")) return { kind: "add", text: line.slice(1) };
  if (line.startsWith("-")) return { kind: "del", text: line.slice(1) };
  if (line.startsWith("\\")) return { kind: "meta", text: line };
  return { kind: "context", text: line.startsWith(" ") ? line.slice(1) : line };
}

function diffFile(path: string, kind: DiffFileKind, movePath: string | null, lines: DiffLine[]): DiffFile {
  return {
    path,
    kind,
    movePath,
    lines,
    added: lines.filter((line) => line.kind === "add").length,
    removed: lines.filter((line) => line.kind === "del").length,
  };
}

/** `fileChange` 아이템의 변경 하나를 화면용으로 바꾼다. */
export function diffFileFromChange(change: FileUpdateChange): DiffFile {
  switch (change.kind.type) {
    case "add":
      return diffFile(change.path, "add", null, splitLines(change.diff).map((text) => ({ kind: "add", text })));
    case "delete":
      return diffFile(change.path, "delete", null, splitLines(change.diff).map((text) => ({ kind: "del", text })));
    case "update":
      return diffFile(change.path, "update", change.kind.move_path, splitLines(change.diff).map(hunkLine));
  }
}

function stripPrefix(path: string): string {
  return path.replace(/^[ab]\//, "");
}

/** git 형식 diff (`turn/diff/updated`) 를 파일별로 나눈다. */
export function parseGitDiff(diff: string): DiffFile[] {
  const files: DiffFile[] = [];
  let header: { from: string; to: string; kind: DiffFileKind; renameTo: string | null } | null = null;
  let lines: DiffLine[] = [];
  let inHunks = false;

  const flush = () => {
    if (!header) return;
    const path = header.kind === "add" ? header.to : header.from;
    const movePath = header.renameTo ?? (header.kind === "update" && header.from !== header.to ? header.to : null);
    files.push(diffFile(path, header.kind, movePath, lines));
  };

  for (const line of splitLines(diff)) {
    const start = /^diff --git a\/(.+) b\/(.+)$/.exec(line);
    if (start) {
      flush();
      header = { from: start[1]!, to: start[2]!, kind: "update", renameTo: null };
      lines = [];
      inHunks = false;
      continue;
    }
    if (!header) continue;
    if (inHunks) {
      lines.push(hunkLine(line));
    } else if (line.startsWith("@@")) {
      inHunks = true;
      lines.push(hunkLine(line));
    } else if (line.startsWith("new file mode")) {
      header.kind = "add";
    } else if (line.startsWith("deleted file mode")) {
      header.kind = "delete";
    } else if (line.startsWith("rename to ")) {
      header.renameTo = line.slice("rename to ".length);
    } else if (line.startsWith("--- ") && line !== "--- /dev/null") {
      header.from = stripPrefix(line.slice(4));
    } else if (line.startsWith("+++ ") && line !== "+++ /dev/null") {
      header.to = stripPrefix(line.slice(4));
    }
  }
  flush();
  return files;
}

/**
 * 턴 전체의 변경이다. `turn/diff/updated` 를 받았으면 그것을 쓴다 (같은 파일의 여러 변경이 합쳐져 있다).
 * 기록에서 연 이전 턴에는 그 알림이 없으므로, 적용된 `fileChange` 아이템의 변경을 차례로 보여 준다.
 */
export function turnDiffFiles(diff: string | null, items: readonly ThreadItem[]): DiffFile[] {
  if (diff !== null) {
    return parseGitDiff(diff);
  }
  return items.flatMap((item) =>
    item.type === "fileChange" && item.status === "completed" ? item.changes.map(diffFileFromChange) : [],
  );
}

export function totalCounts(files: readonly DiffFile[]): { added: number; removed: number } {
  return files.reduce(
    (total, file) => ({ added: total.added + file.added, removed: total.removed + file.removed }),
    { added: 0, removed: 0 },
  );
}
