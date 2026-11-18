// 파일 변경 표시이다 (R1). 줄은 파일을 펼쳤을 때만 그린다. 긴 대화에 변경이 수백 개 있어도 가볍게 유지한다.
import { Component, lazy, Suspense, useState, type ReactNode } from "react";

import { type DiffFile, type DiffLine, patchText } from "./diff";
import { setDiffStyle, useDiffStyle } from "./diffStyle";

const PatchDiffView = lazy(() => import("./PatchDiffView"));

/** 이 줄 수를 넘는 파일은 줄을 그대로 그린다. 문법 색과 좌우 비교의 계산이 화면을 멈추지 않게 한다. */
const PATCH_VIEW_LINE_LIMIT = 5000;

/** 처음에 그리는 줄 수이다. 더 긴 파일은 버튼을 눌러 나머지를 그린다. */
const INITIAL_LINE_LIMIT = 1000;

const KIND_LABEL: Record<DiffFile["kind"], string> = { add: "추가", delete: "삭제", update: "수정" };
const LINE_MARKER: Record<DiffLine["kind"], string> = { add: "+", del: "-", context: " ", hunk: "", meta: "" };

export function DiffCounts({ added, removed }: { added: number; removed: number }) {
  return (
    <span className="diff-counts">
      <span className="diff-count-add">+{added}</span> <span className="diff-count-del">-{removed}</span>
    </span>
  );
}

export function DiffLines({ lines }: { lines: DiffLine[] }) {
  const [showAll, setShowAll] = useState(false);
  const shown = showAll ? lines : lines.slice(0, INITIAL_LINE_LIMIT);
  return (
    <div className="diff-lines" data-testid="diff-lines">
      {shown.map((line, index) => (
        <div key={index} className={`diff-line diff-${line.kind}`}>
          <span className="diff-marker">{LINE_MARKER[line.kind]}</span>
          {line.text}
        </div>
      ))}
      {shown.length < lines.length && (
        <button type="button" className="diff-more" onClick={() => setShowAll(true)}>
          나머지 {lines.length - shown.length}줄 보기
        </button>
      )}
    </div>
  );
}

/** 새 표시가 그리다가 실패하면 줄을 그대로 그린 것을 보인다. diff 하나의 실패가 대화 화면을 지우지 않게 한다. */
class PatchBoundary extends Component<{ fallback: ReactNode; children: ReactNode }, { failed: boolean }> {
  override state = { failed: false };

  static getDerivedStateFromError(): { failed: boolean } {
    return { failed: true };
  }

  override render(): ReactNode {
    return this.state.failed ? this.props.fallback : this.props.children;
  }
}

/**
 * 파일 하나의 변경이다. 문법 색과 좌우 비교가 있는 표시로 그린다 (P-12).
 * 줄 번호가 없는 변경, 아주 긴 변경, 표시를 불러오는 동안과 실패한 때는 줄을 그대로 그린다.
 */
export function DiffBody({ file }: { file: DiffFile }) {
  const plain = <DiffLines lines={file.lines} />;
  const patch = file.lines.length <= PATCH_VIEW_LINE_LIMIT ? patchText(file) : null;
  if (patch === null) return plain;
  return (
    <PatchBoundary fallback={plain}>
      <Suspense fallback={plain}>
        <PatchDiffView patch={patch} fallback={plain} />
      </Suspense>
    </PatchBoundary>
  );
}

/** 좌우 비교와 한 줄 보기를 바꾼다. 모든 diff 에 적용된다. */
export function DiffStyleToggle() {
  const style = useDiffStyle();
  return (
    <button
      type="button"
      className="button diff-style-toggle"
      aria-pressed={style === "split"}
      onClick={() => setDiffStyle(style === "split" ? "unified" : "split")}
      data-testid="diff-style-toggle"
    >
      {style === "split" ? "한 줄로 보기" : "좌우로 보기"}
    </button>
  );
}

function DiffFileView({ file, defaultOpen }: { file: DiffFile; defaultOpen: boolean }) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <details className="diff-file" data-testid="diff-file" open={open} onToggle={(event) => setOpen(event.currentTarget.open)}>
      <summary>
        <span className={`diff-kind diff-kind-${file.kind}`}>{file.movePath ? "이동" : KIND_LABEL[file.kind]}</span>
        <code>{file.path}</code>
        {file.movePath && (
          <>
            {" → "}
            <code>{file.movePath}</code>
          </>
        )}
        <DiffCounts added={file.added} removed={file.removed} />
      </summary>
      {open && <DiffBody file={file} />}
    </details>
  );
}

export function DiffFileList({ files, defaultOpen = false }: { files: readonly DiffFile[]; defaultOpen?: boolean }) {
  return (
    <div className="diff-file-list">
      {files.map((file, index) => (
        <DiffFileView key={`${index}:${file.path}`} file={file} defaultOpen={defaultOpen} />
      ))}
    </div>
  );
}
