// `@pierre/diffs` 로 그리는 diff 이다 (P-12 1단계: 보기). 문법 색과 좌우 비교가 있다. 편집과 Worker 는 쓰지 않는다.
// 이 파일은 필요할 때만 불러온다 (`DiffView` 의 `lazy`). 그래서 라이브러리는 첫 화면의 묶음에 들어가지 않는다.
import { getFiletypeFromFileName, getSingularPatch } from "@pierre/diffs";
import { FileDiff } from "@pierre/diffs/react";
import { useMemo, type ReactNode } from "react";

import { useUiStore } from "../session/uiStore";
import { bundledLanguage } from "./diffLanguages";
import { useDiffStyle } from "./diffStyle";

/** `patch` 를 읽지 못하면 `fallback` (줄을 그대로 그린 것)을 보인다. */
export default function PatchDiffView({ patch, fallback }: { patch: string; fallback: ReactNode }) {
  const theme = useUiStore((state) => state.theme);
  const diffStyle = useDiffStyle();
  const fileDiff = useMemo(() => {
    try {
      const parsed = getSingularPatch(patch);
      return parsed.hunks.length > 0 ? { ...parsed, lang: bundledLanguage(getFiletypeFromFileName(parsed.name)) } : null;
    } catch {
      return null;
    }
  }, [patch]);
  const options = useMemo(
    // 파일 이름과 줄 수는 접는 줄(`summary`)이 이미 보인다. 그래서 라이브러리의 파일 머리는 끈다.
    // 덩어리 사이의 줄은 글 없는 선이다. patch 는 파일의 일부라서 "바뀌지 않은 줄 N개"를 펼칠 수 없다.
    () => ({ diffStyle, themeType: theme, disableFileHeader: true, overflow: "scroll" as const, hunkSeparators: "simple" as const }),
    [diffStyle, theme],
  );
  if (!fileDiff) return fallback;
  return (
    <div className="diff-lines diff-patch" data-testid="diff-lines" data-diff-style={diffStyle}>
      <FileDiff fileDiff={fileDiff} options={options} disableWorkerPool />
    </div>
  );
}
