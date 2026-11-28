import { useEffect, useMemo, useState } from "react";
import type { GitDiffSide } from "../../shared/gitDiffContext";
import type { TurnDiffFileSource } from "./diff";
import { captureTurnDiff, readTurnDiffContext, type TurnCaptureResult, type TurnContextResult, type TurnDiffCaptureSource, type TurnDiffScope } from "./turnDiffSnapshot";

export function TurnDiffContext({ source, scope }: { readonly source: TurnDiffFileSource; readonly scope: TurnDiffScope }) {
  const { origin, input: { kind, text }, file: { path, movePath } } = source;
  // Streaming message deltas replace display objects without changing the recorded source.
  const captureSource = useMemo<TurnDiffCaptureSource>(() => ({ origin, input: { kind, text }, file: { path, movePath } }),
    [origin, kind, text, path, movePath]);
  const [captured, setCaptured] = useState<{
    readonly source: TurnDiffCaptureSource; readonly scope: TurnDiffScope; readonly result: TurnCaptureResult;
  } | null>(null);
  const capture = captured?.source === captureSource && captured.scope === scope ? captured.result : null;
  const [result, setResult] = useState<TurnContextResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [side, setSide] = useState<GitDiffSide>("new");
  const [start, setStart] = useState<number | "">(1);
  const [end, setEnd] = useState<number | "">(1);
  useEffect(() => {
    let active = true;
    setCaptured(null); setResult(null); setError(null);
    void captureTurnDiff(captureSource, scope).then(value => {
      if (!active) return;
      setCaptured({ source: captureSource, scope, result: value });
      if (value.status === "captured") {
        const initial = value.snapshot.new.firstLine !== null ? "new" : "old";
        setSide(initial);
        setStart(value.snapshot[initial].firstLine ?? 1);
        setEnd(value.snapshot[initial].firstLine ?? 1);
      }
    }, (failure: unknown) => {
      if (active) setError(failure instanceof Error ? failure.message : String(failure));
    });
    return () => { active = false; };
  }, [captureSource, scope]);
  const valid = typeof start === "number" && typeof end === "number" &&
    Number.isSafeInteger(start) && Number.isSafeInteger(end) && start >= 1 && end >= start;
  const read = () => {
    if (capture?.status === "captured" && valid) setResult(readTurnDiffContext(capture.snapshot, {
      snapshotId: capture.snapshot.snapshotId, side, startLine: start, endLine: end,
    }));
  };
  return <section className="diff-hint" data-testid="turn-source">
    <p>턴 제공 원문 · {scope.turnId}</p>
    {capture?.status === "captured" && <>
      <details><summary>출처 · snapshot</summary><code data-testid="turn-source-id">{capture.snapshot.snapshotId}</code></details>
      <p>{capture.snapshot.old.status === "complete" && capture.snapshot.new.status === "complete"
        ? "제공된 파일 원문" : "부분 patch · 제공되지 않은 앞뒤 문맥은 unavailable"}</p>
      <div className="review-toolbar" style={{ flexWrap: "wrap" }}>
        <label>면 <select aria-label="턴 원문 면" value={side} onChange={event => {
          const value = event.target.value;
          if (value === "old" || value === "new") { setSide(value); setResult(null); }
        }}><option value="old">이전</option><option value="new">이후</option></select></label>
        <label>시작 <input aria-label="턴 원문 시작 행" type="number" min={1} value={start} style={{ width: 70 }}
          onChange={event => { setStart(Number.isFinite(event.target.valueAsNumber) ? event.target.valueAsNumber : ""); setResult(null); }} /></label>
        <label>끝 <input aria-label="턴 원문 끝 행" type="number" min={1} value={end} style={{ width: 70 }}
          onChange={event => { setEnd(Number.isFinite(event.target.valueAsNumber) ? event.target.valueAsNumber : ""); setResult(null); }} /></label>
        <button type="button" className="button" disabled={!valid} onClick={read}>턴 원문 범위 읽기</button>
      </div>
      {result?.status === "available" && <pre data-testid="turn-source-excerpt" style={{ overflow: "auto", maxHeight: 300 }}>{result.text}</pre>}
      {result?.status === "unavailable" && <p role="status">unavailable: {result.reason} · 현재 HEAD/파일로 채우지 않습니다.</p>}
    </>}
    {capture?.status === "unavailable" && <p role="status">unavailable: {capture.reason}</p>}
    {error && <p role="alert">원문 snapshot을 만들 수 없습니다: {error}</p>}
    {capture === null && error === null && <p role="status">턴 원문을 고정하는 중…</p>}
  </section>;
}
