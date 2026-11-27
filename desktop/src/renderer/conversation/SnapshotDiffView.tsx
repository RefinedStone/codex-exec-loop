import { useCallback, useEffect, useMemo, useState } from "react";
import type { SelectedLineRange } from "@pierre/diffs";
import type { AkraBridge, GitDiffSnapshot } from "../../shared/bridge";
import type { GitDiffSide } from "../../shared/gitDiffContext";
import type { DiffFile } from "./diff";
import { diffCoordinates, selectionAt, type DiffCoordinate, type DiffSelection } from "./diffCoordinates";
import { DiffBody } from "./DiffView";
import { CONTEXT_REASON, SnapshotContext } from "./SnapshotContext";
import "./snapshotDiff.css";

/** #2827 consumes source/side/file coordinates through onSelectionChange. This surface never sends a message. */
export function SnapshotDiffView({ cwd, file, snapshot, git, selection: controlledSelection, onSelectionChange }: {
  readonly cwd: string; readonly file: DiffFile; readonly snapshot: GitDiffSnapshot;
  readonly git: Pick<AkraBridge, "gitDiffContext">;
  readonly selection?: DiffSelection | null;
  readonly onSelectionChange?: (selection: DiffSelection | null) => void;
}) {
  const [localSelection, setSelection] = useState<DiffSelection | null>(null);
  const selection = controlledSelection === undefined ? localSelection : controlledSelection;
  const [side, setSide] = useState<GitDiffSide>("new");
  const [start, setStart] = useState<number | "">(1);
  const [end, setEnd] = useState<number | "">(1);
  const [query, setQuery] = useState("");
  const [matchIndex, setMatchIndex] = useState(-1);
  const [hunkIndex, setHunkIndex] = useState(-1);
  const [wrap, setWrap] = useState(false);
  const [whitespace, setWhitespace] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [revealSelection, setRevealSelection] = useState(false);
  useEffect(() => {
    if (controlledSelection) {
      setSide(controlledSelection.side); setStart(controlledSelection.startLine); setEnd(controlledSelection.endLine);
    }
  }, [controlledSelection]);
  const coordinates = useMemo(() => diffCoordinates(file), [file]);
  const hunks = useMemo(() => coordinates.filter((row) => row.hunk), [coordinates]);
  const matches = useMemo(() => query.trim() ? coordinates.filter((row) => !row.hunk &&
    (side === "old" ? row.oldLine : row.newLine) !== null && row.text.toLocaleLowerCase().includes(query.toLocaleLowerCase())) : [], [coordinates, query, side]);
  const choose = useCallback((value: DiffSelection | null, reveal = true) => {
    if (value) {
      const info = snapshot.contents?.[value.side];
      if (info?.status !== "complete" || value.startLine < 1 || value.endLine > info.lineCount ||
        value.endLine - value.startLine >= 200) {
        setNotice("확인 가능한 원문의 한 면에서 200행 이내로 선택하십시오.");
        return;
      }
    }
    setSelection(value); setRevealSelection(reveal); onSelectionChange?.(value); setNotice(null);
    if (value) { setSide(value.side); setStart(value.startLine); setEnd(value.endLine); }
  }, [snapshot, onSelectionChange]);
  const visit = (row: DiffCoordinate | undefined) => {
    if (!row) return;
    const preferred = side === "old" ? row.oldLine : row.newLine;
    const line = preferred ?? row.newLine ?? row.oldLine;
    if (line === null) return;
    const targetSide = preferred !== null ? side : row.newLine !== null ? "new" : "old";
    choose(selectionAt(snapshot, { side: targetSide, startLine: line, endLine: line }));
  };
  const move = (kind: "hunk" | "match", delta: number) => {
    const rows = kind === "hunk" ? hunks : matches;
    if (rows.length === 0) return;
    const previous = kind === "hunk" ? hunkIndex : matchIndex;
    const index = previous < 0 ? (delta > 0 ? 0 : rows.length - 1) : (previous + delta + rows.length) % rows.length;
    if (kind === "hunk") setHunkIndex(index); else setMatchIndex(index);
    visit(rows[index]);
  };
  const fromEngine = useCallback((range: SelectedLineRange | null) => {
    if (range === null) return choose(null, false);
    if (range.endSide && range.side !== range.endSide) { setNotice("한 면의 연속 범위를 선택하십시오."); return; }
    choose(selectionAt(snapshot, { side: range.side === "deletions" ? "old" : "new",
      startLine: Math.min(range.start, range.end), endLine: Math.max(range.start, range.end) }), false);
  }, [snapshot, choose]);
  const selectedLines = useMemo<SelectedLineRange | null>(() => selection ? {
    side: selection.side === "old" ? "deletions" : "additions",
    start: selection.startLine, end: selection.endLine,
  } : null, [selection]);
  const interaction = useMemo(() => ({ selectedLines, onLineSelected: fromEngine, wrap }), [selectedLines, fromEngine, wrap]);
  const rangeIsValid = typeof start === "number" && typeof end === "number" &&
    Number.isSafeInteger(start) && Number.isSafeInteger(end) && start >= 1 && end >= start;
  if (selection && (selection.snapshotId !== snapshot.snapshotId || selection.source !== snapshot.source ||
    selection.path !== snapshot.path)) {
    return <p role="status">다른 snapshot의 선택입니다. 원래 파일과 snapshot을 여십시오.</p>;
  }
  return <section className="snapshot-diff" data-testid="snapshot-diff">
    <div className="review-toolbar snapshot-toolbar">
      <button type="button" className="button" onClick={() => move("hunk", -1)} disabled={!hunks.length}>이전 hunk</button>
      <button type="button" className="button" onClick={() => move("hunk", 1)} disabled={!hunks.length}>다음 hunk</button>
      <span>{hunkIndex + 1}/{hunks.length}</span>
      <input aria-label="읽은 파일 변경 내용 찾기" placeholder="읽은 파일 변경 내 찾기" value={query}
        onChange={(event) => { setQuery(event.target.value); setMatchIndex(-1); }} />
      <button type="button" className="button" onClick={() => move("match", -1)} disabled={!matches.length}>이전 결과</button>
      <button type="button" className="button" onClick={() => move("match", 1)} disabled={!matches.length}>다음 결과</button>
      <span className="muted">{matches.length}개 · 이 파일의 읽은 patch만 검색</span>
    </div>
    <div className="review-toolbar snapshot-toolbar">
      <label>면 <select aria-label="선택 면" value={side} onChange={(event) => {
        const value = event.target.value; if (value === "old" || value === "new") setSide(value);
      }}><option value="old">이전</option><option value="new">이후</option></select></label>
      <label>시작 <input aria-label="선택 시작 행" type="number" min={1} value={start} onChange={(event) => setStart(Number.isFinite(event.target.valueAsNumber) ? event.target.valueAsNumber : "")} /></label>
      <label>끝 <input aria-label="선택 끝 행" type="number" min={start || 1} value={end} onChange={(event) => setEnd(Number.isFinite(event.target.valueAsNumber) ? event.target.valueAsNumber : "")} /></label>
      <button type="button" className="button" disabled={!rangeIsValid}
        onClick={() => { if (rangeIsValid) choose(selectionAt(snapshot, { side, startLine: start, endLine: end })); }}>원문 범위 보기</button>
      <label><input type="checkbox" checked={wrap} onChange={(event) => setWrap(event.target.checked)} />줄바꿈</label>
      <label><input type="checkbox" checked={whitespace} onChange={(event) => setWhitespace(event.target.checked)} />원문 공백 표시</label>
    </div>
    {(["old", "new"] as const).map((contentSide) => {
      const info = snapshot.contents?.[contentSide];
      return info?.status === "unavailable" && <p key={contentSide} role="status">
        {contentSide === "old" ? "이전" : "이후"} 원문 미제공: {CONTEXT_REASON[info.reason]}
      </p>;
    })}
    {notice && <p role="status">{notice}</p>}
    <DiffBody file={file} interaction={interaction} />
    {selection && <SnapshotContext key={snapshot.snapshotId} cwd={cwd} snapshot={snapshot} selection={selection} git={git}
      whitespace={whitespace} reveal={revealSelection || controlledSelection !== undefined} />}
  </section>;
}
