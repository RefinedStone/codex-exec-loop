import { useEffect, useState } from "react";

import { JsonRpcError } from "../rpc/jsonRpcClient";

type StoredState = "cancelled" | "diff_too_large" | "source_unavailable" | "backend_failed" | "result_invalid";
export interface StoredPrReview {
  head: string;
  result: { status: StoredState } | {
    status: "reviewed";
    backend: string;
    result: { summary: string; findings: { path: string; line: number; severity: string; body: string }[] };
  } | null;
}

const STATUS_LABEL: Record<StoredState, string> = {
  cancelled: "취소됨",
  diff_too_large: "변경 내용이 너무 큽니다.",
  source_unavailable: "변경 내용을 읽지 못했습니다.",
  backend_failed: "검토 백엔드가 실패했습니다.",
  result_invalid: "검토 결과 형식이 올바르지 않습니다.",
};

export type PrReviewMode = "off" | "observe" | "on";

export function PrReviewResult({ number, read }: {
  number: number;
  read(number: number): Promise<{ review: StoredPrReview | null; mode: PrReviewMode }>;
}) {
  const [state, setState] = useState<{ review: StoredPrReview | null; mode: PrReviewMode } | { error: string } | "loading" | "unsupported">("loading");
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    let current = true;
    setState("loading");
    void read(number).then(
      (value) => { if (current) setState(value); },
      (error: unknown) => {
        if (current) setState(error instanceof JsonRpcError && (error.code === -32601 || error.code === -32602)
          ? "unsupported" : { error: "저장된 검토 결과를 읽지 못했습니다." });
      },
    );
    return () => { current = false; };
  }, [number, read, revision]);
  if (state === "unsupported") return null;
  const stored = typeof state === "object" && "review" in state ? state.review : null;
  const result = stored?.result;
  return (
    <section className="work-section" data-testid="pr-review-result">
      <div className="work-header">
        <h3>PR #{number} 위임 검토</h3>
        <button type="button" onClick={() => setRevision((value) => value + 1)} disabled={state === "loading"}>검토 새로 고침</button>
      </div>
      {state === "loading" && <p className="muted" role="status">검토 결과 읽는 중…</p>}
      {typeof state === "object" && "error" in state && <p className="error" role="alert">{state.error}</p>}
      {typeof state === "object" && "mode" in state && <p className="work-meta muted">저장소 설정 · {state.mode}</p>}
      {typeof state === "object" && "review" in state && !stored && <p className="muted">저장된 검토가 없습니다.</p>}
      {stored && <p className="work-meta muted" title={stored.head}>검토 커밋 <code>{stored.head.slice(0, 12)}</code></p>}
      {stored && !result && <p className="muted">아직 검토 결과가 없습니다.</p>}
      {result && result.status !== "reviewed" && <p className="muted">{STATUS_LABEL[result.status]}</p>}
      {result?.status === "reviewed" && <>
        <p className="work-meta muted">{result.backend}</p>
        <p className="delegation-text">{result.result.summary}</p>
        {result.result.findings.length === 0 ? <p className="muted">발견 사항이 없습니다.</p> : (
          <ul>{result.result.findings.map((item, index) => (
            <li className="work-item" key={index}>
              <strong>{item.severity}</strong> · <code className="delegation-text">{item.path}:{item.line}</code>
              <p className="delegation-text">{item.body}</p>
            </li>
          ))}</ul>
        )}
      </>}
    </section>
  );
}
