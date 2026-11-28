import { useEffect, useRef, useState } from "react";
import type { AkraBridge, GitDiffSnapshot } from "../../shared/bridge";
import type { GitDiffContextResult } from "../../shared/gitDiffContext";
import type { DiffSelection } from "./diffCoordinates";

export const CONTEXT_REASON = {
  size_limit: "파일 크기 제한 (면당 1 MiB)",
  binary: "바이너리 원문",
  unsupported_entry: "일반 텍스트 파일이 아닌 항목 (submodule·링크 등)",
  invalid_utf8: "UTF-8 원문을 확인할 수 없음",
  snapshot_expired: "원문 cache 소실 · 새로 읽어 다시 선택하십시오",
  scope_mismatch: "다른 저장소 또는 비교 기준",
  range_limit: "요청 한도 초과 (200행·64 KiB) 또는 없는 행",
} as const;

export function SnapshotContext({ cwd, snapshot, selection, git, whitespace, reveal }: {
  readonly cwd: string; readonly snapshot: GitDiffSnapshot; readonly selection: DiffSelection;
  readonly git: Pick<AkraBridge, "gitDiffContext">;
  readonly whitespace: boolean;
  readonly reveal: boolean;
}) {
  const [padding, setPadding] = useState(10);
  const [state, setState] = useState<GitDiffContextResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const panel = useRef<HTMLDivElement>(null);
  const info = snapshot.contents?.[selection.side];
  useEffect(() => {
    setPadding(10);
  }, [selection.side, selection.startLine, selection.endLine]);
  useEffect(() => {
    setState(null); setError(null);
    if (info?.status !== "complete" || info.lineCount === 0) return;
    let active = true;
    const contextPadding = Math.min(padding, Math.floor((200 - (selection.endLine - selection.startLine + 1)) / 2));
    const startLine = Math.max(1, selection.startLine - contextPadding);
    const endLine = Math.min(info.lineCount, selection.endLine + contextPadding);
    git.gitDiffContext(cwd, { snapshotId: snapshot.snapshotId, source: snapshot.source,
      side: selection.side, startLine, endLine }).then((result) => {
      if (!active) return;
      if (result.status === "available" && (result.snapshotId !== snapshot.snapshotId ||
        result.source !== snapshot.source || result.side !== selection.side ||
        result.startLine !== startLine || result.endLine !== endLine)) return;
      setState(result);
    }, (failure: unknown) => {
      if (active) setError(failure instanceof Error ? failure.message : String(failure));
    });
    return () => { active = false; };
  }, [cwd, snapshot.snapshotId, snapshot.source, info, selection.side, selection.startLine, selection.endLine, padding, git]);
  useEffect(() => {
    if (reveal && state?.status === "available") {
      panel.current?.querySelector(`[data-context-line="${selection.startLine}"]`)?.scrollIntoView({ block: "center" });
    }
  }, [state, selection.startLine, reveal]);

  const reason = info?.status === "unavailable" ? CONTEXT_REASON[info.reason]
    : state?.status === "unavailable" ? CONTEXT_REASON[state.reason] : null;
  const lines = state?.status === "available" ? state.text.split("\n") : [];
  if (lines.at(-1) === "") lines.pop();
  return <div ref={panel} className="snapshot-context" data-testid="snapshot-context">
    <div className="review-toolbar">
      <strong>{selection.side === "old" ? "이전 면" : "이후 면"} · {selection.startLine}–{selection.endLine}</strong>
      <span className="muted">원래 snapshot · {snapshot.snapshotId.slice(0, 12)}</span>
      <button type="button" className="button" disabled={info?.status !== "complete" ||
        selection.endLine - selection.startLine + 1 + (padding + 10) * 2 > 200}
        onClick={() => setPadding((value) => value + 10)}>앞뒤 10행 더 보기</button>
    </div>
    {info === undefined && <p>원문 미제공 · 과거 patch를 현재 파일로 채우지 않습니다.</p>}
    {info?.status === "complete" && info.lineCount === 0 && <p>이 면에는 파일 행이 없습니다.</p>}
    {reason && <p role="status">원문을 더 불러올 수 없음: {reason}</p>}
    {error && <p role="alert">{error}</p>}
    {info?.status === "complete" && info.lineCount > 0 && state === null && !error && <p>원래 문맥을 읽는 중…</p>}
    {state?.status === "available" && <>
      {!state.current && <p role="status">선택 후 변경됨 · 원래 코드를 유지합니다.</p>}
      <pre>{lines.map((text, index) => {
        const line = state.startLine + index;
        return <div key={line} data-context-line={line} data-selected={line >= selection.startLine && line <= selection.endLine}>
          <span className="snapshot-line-number">{line}</span><code>{whitespace
            ? text.replace(/\r$/, "").replace(/ /g, "\u00b7").replace(/\t/g, "\u2192   ")
            : text.replace(/\r$/, "")}</code>
        </div>;
      })}</pre>
    </>}
  </div>;
}
