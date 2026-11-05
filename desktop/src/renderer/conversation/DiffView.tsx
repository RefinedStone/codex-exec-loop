// 파일 변경 표시이다 (R1). 줄은 파일을 펼쳤을 때만 그린다. 긴 대화에 변경이 수백 개 있어도 가볍게 유지한다.
import { useState } from "react";

import type { DiffFile, DiffLine } from "./diff";
import { totalCounts } from "./diff";

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
      {open && <DiffLines lines={file.lines} />}
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

/** 턴 끝의 "이 턴의 변경" 요약이다. 펼치면 파일 목록이 나온다. */
export function TurnChanges({ files }: { files: readonly DiffFile[] }) {
  const counts = totalCounts(files);
  return (
    <details className="turn-changes" data-testid="turn-changes">
      <summary>
        이 턴의 변경 · 파일 {files.length}개 <DiffCounts added={counts.added} removed={counts.removed} />
      </summary>
      <DiffFileList files={files} />
    </details>
  );
}
