// 파일 변경 표시이다 (R1). 줄은 파일을 펼쳤을 때만 그린다. 긴 대화에 변경이 수백 개 있어도 가볍게 유지한다.
import { Component, lazy, Suspense, useState, type ReactNode } from "react";
import type { SelectedLineRange } from "@pierre/diffs";

import { type DiffFile, type DiffLine, patchText } from "./diff";
import { setDiffStyle, useDiffStyle } from "./diffStyle";

const PatchDiffView = lazy(() => import("./PatchDiffView"));

export interface DiffInteraction {
  readonly selectedLines: SelectedLineRange | null;
  readonly onLineSelected: (range: SelectedLineRange | null) => void;
  readonly wrap: boolean;
}

/** 이 줄 수를 넘는 파일은 줄을 그대로 그린다. 문법 색과 좌우 비교의 계산이 화면을 멈추지 않게 한다. */
const PATCH_VIEW_LINE_LIMIT = 5000;

/** 한 페이지의 표시 상한이다. 큰 파일도 전체 행을 한 번에 DOM에 넣지 않는다. */
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

export function DiffLines({ lines, wrap = false }: { lines: DiffLine[]; wrap?: boolean }) {
  const [offset, setOffset] = useState(0);
  const shown = lines.slice(offset, offset + INITIAL_LINE_LIMIT);
  return (
    <div className="diff-lines" data-testid="diff-lines">
      {shown.map((line, index) => (
        <div key={offset + index} className={`diff-line diff-${line.kind}`}
          style={{ whiteSpace: wrap ? "pre-wrap" : "pre", minWidth: wrap ? 0 : "max-content", overflowWrap: wrap ? "anywhere" : "normal" }}>
          <span className="diff-marker">{LINE_MARKER[line.kind]}</span>
          {line.text}
        </div>
      ))}
      {lines.length > INITIAL_LINE_LIMIT && <div className="review-toolbar">
        <button type="button" className="diff-more" disabled={offset === 0}
          onClick={() => setOffset((value) => Math.max(0, value - INITIAL_LINE_LIMIT))}>이전 표시 구간</button>
        <span>{offset + 1}–{offset + shown.length}/{lines.length} 표시 행 · 문법 색 생략</span>
        <button type="button" className="diff-more" disabled={offset + shown.length >= lines.length}
          onClick={() => setOffset((value) => value + INITIAL_LINE_LIMIT)}>다음 표시 구간</button>
      </div>}
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
export function DiffBody({ file, interaction }: { file: DiffFile; interaction?: DiffInteraction }) {
  const plain = <DiffLines key={file.path} lines={file.lines} wrap={interaction?.wrap} />;
  const patch = file.lines.length <= PATCH_VIEW_LINE_LIMIT ? patchText(file) : null;
  if (patch === null) return plain;
  return (
    <PatchBoundary fallback={plain}>
      <Suspense fallback={plain}>
        <PatchDiffView patch={patch} fallback={plain} interaction={interaction} />
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

export function DiffFileView({ file, defaultOpen, context }: { file: DiffFile; defaultOpen: boolean; context?: ReactNode }) {
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
      {open && <><DiffBody file={file} />{context}</>}
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
