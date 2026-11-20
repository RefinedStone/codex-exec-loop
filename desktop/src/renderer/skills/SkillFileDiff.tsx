// 스킬 파일 하나의 이전 내용과 새 내용의 차이이다 (#2383). `@pierre/diffs` 는 필요할 때만 불러온다 (`SkillsPanel` 의 `lazy`).
import { getFiletypeFromFileName, parseDiffFromFile } from "@pierre/diffs";
import { FileDiff } from "@pierre/diffs/react";
import { useMemo } from "react";

import { bundledLanguage } from "../conversation/diffLanguages";
import { useDiffStyle } from "../conversation/diffStyle";
import { useUiStore } from "../session/uiStore";

export default function SkillFileDiff({ path, oldText, newText }: { path: string; oldText: string; newText: string }) {
  const theme = useUiStore((state) => state.theme);
  const diffStyle = useDiffStyle();
  const fileDiff = useMemo(() => {
    try {
      return { ...parseDiffFromFile({ name: path, contents: oldText }, { name: path, contents: newText }), lang: bundledLanguage(getFiletypeFromFileName(path)) };
    } catch {
      return null;
    }
  }, [path, oldText, newText]);
  const options = useMemo(() => ({ diffStyle, themeType: theme, disableFileHeader: true, overflow: "scroll" as const, hunkSeparators: "simple" as const }), [diffStyle, theme]);
  if (!fileDiff) return <p className="muted">차이를 그리지 못했습니다.</p>;
  return (
    <div className="diff-lines diff-patch" data-testid="skill-diff" data-diff-style={diffStyle}>
      <FileDiff fileDiff={fileDiff} options={options} disableWorkerPool />
    </div>
  );
}
